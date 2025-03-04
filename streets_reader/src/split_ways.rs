use std::collections::{btree_map::Entry, BTreeMap, HashMap};

use abstutil::{Counter, Tags, Timer};
use geom::{Distance, HashablePt2D, PolyLine, Pt2D};
use osm2streets::{
    osm, Direction, IntersectionControl, IntersectionID, IntersectionKind, OriginalRoad, Road,
    RoadID, StreetNetwork,
};

use super::OsmExtract;

/// Returns a mapping of all points to the split road. Some internal points on roads get removed
/// here, so this mapping isn't redundant.
pub fn split_up_roads(
    streets: &mut StreetNetwork,
    mut input: OsmExtract,
    timer: &mut Timer,
) -> HashMap<HashablePt2D, RoadID> {
    timer.start("splitting up roads");

    let mut roundabout_centers: Vec<(Pt2D, Vec<osm::NodeID>)> = Vec::new();
    // Note we iterate over this later and assign IDs based on the order, so HashMap would be
    // non-deterministic
    let mut pt_to_intersection: BTreeMap<HashablePt2D, osm::NodeID> = BTreeMap::new();

    input.roads.retain(|(id, pts, tags)| {
        if should_collapse_roundabout(pts, tags) {
            info!("Collapsing tiny roundabout {}", id);

            let ids: Vec<osm::NodeID> = pts
                .iter()
                .map(|pt| input.osm_node_ids[&pt.to_hashable()])
                .collect();

            // Arbitrarily use the first node's ID
            // TODO Test more carefully after opaque IDs
            let id = ids[0];

            for pt in pts {
                pt_to_intersection.insert(pt.to_hashable(), id);
            }

            roundabout_centers.push((Pt2D::center(pts), ids));

            false
        } else {
            true
        }
    });

    let mut counts_per_pt = Counter::new();
    for (_, pts, _) in &input.roads {
        for (idx, raw_pt) in pts.iter().enumerate() {
            let pt = raw_pt.to_hashable();
            let count = counts_per_pt.inc(pt);

            // All start and endpoints of ways are also intersections.
            if count == 2 || idx == 0 || idx == pts.len() - 1 {
                if let Entry::Vacant(e) = pt_to_intersection.entry(pt) {
                    let id = input.osm_node_ids[&pt];
                    e.insert(id);
                }
            }
        }
    }

    let mut osm_id_to_id: HashMap<osm::NodeID, IntersectionID> = HashMap::new();
    for (pt, osm_id) in &pt_to_intersection {
        let id = streets.insert_intersection(
            vec![*osm_id],
            pt.to_pt2d(),
            // Assume a complicated intersection, until we determine otherwise.
            IntersectionKind::Intersection,
            if input.traffic_signals.remove(pt).is_some() {
                IntersectionControl::Signalled
            } else {
                // TODO default to uncontrolled, guess StopSign as a transform
                IntersectionControl::Signed
            },
        );
        osm_id_to_id.insert(*osm_id, id);
    }

    // Set roundabouts to their center
    for (pt, osm_ids) in roundabout_centers {
        let id = streets.insert_intersection(
            osm_ids.clone(),
            pt,
            IntersectionKind::Intersection,
            IntersectionControl::Signed,
        );
        for osm_id in osm_ids {
            osm_id_to_id.insert(osm_id, id);
        }
    }

    let mut pt_to_road: HashMap<HashablePt2D, RoadID> = HashMap::new();

    // Now actually split up the roads based on the intersections
    timer.start_iter("split roads", input.roads.len());
    for (osm_way_id, orig_pts, orig_tags) in &input.roads {
        timer.next();
        let mut tags = orig_tags.clone();
        let mut pts = Vec::new();
        let endpt1 = pt_to_intersection[&orig_pts[0].to_hashable()];
        let mut i1 = endpt1;

        for pt in orig_pts {
            pts.push(*pt);
            if pts.len() == 1 {
                continue;
            }
            if let Some(i2) = pt_to_intersection.get(&pt.to_hashable()) {
                let id = streets.next_road_id();

                // Note we populate this before simplify_linestring, so even if some points are
                // removed, we can still associate them to the road.
                for (idx, pt) in pts.iter().enumerate() {
                    if idx != 0 && idx != pts.len() - 1 {
                        pt_to_road.insert(pt.to_hashable(), id);
                    }
                }

                let untrimmed_center_line = simplify_linestring(std::mem::take(&mut pts));
                match PolyLine::new(untrimmed_center_line) {
                    Ok(pl) => {
                        streets.roads.insert(
                            id,
                            Road::new(
                                id,
                                vec![OriginalRoad {
                                    osm_way_id: *osm_way_id,
                                    i1,
                                    i2: *i2,
                                }],
                                osm_id_to_id[&i1],
                                osm_id_to_id[i2],
                                pl,
                                tags,
                                &streets.config,
                            ),
                        );
                        streets
                            .intersections
                            .get_mut(&osm_id_to_id[&i1])
                            .unwrap()
                            .roads
                            .push(id);
                        streets
                            .intersections
                            .get_mut(&osm_id_to_id[i2])
                            .unwrap()
                            .roads
                            .push(id);
                    }
                    Err(err) => {
                        error!("Skipping {id}: {err}");
                        // There may be an orphaned intersection left around; a later
                        // transformation should clean it up
                    }
                }

                // Start a new road
                tags = orig_tags.clone();
                i1 = *i2;
                pts.push(*pt);
            }
        }
        assert!(pts.len() == 1);
    }

    // Resolve simple turn restrictions (via a node)
    let mut restrictions = Vec::new();
    for (restriction, from_osm, via_osm, to_osm) in input.simple_turn_restrictions {
        // A via node might not be an intersection
        let via_id = if let Some(x) = osm_id_to_id.get(&via_osm) {
            *x
        } else {
            continue;
        };
        if !streets.intersections.contains_key(&via_id) {
            continue;
        }
        let roads = streets.roads_per_intersection(via_id);
        // If some of the roads are missing, they were likely filtered out -- usually service
        // roads.
        if let (Some(from), Some(to)) = (
            roads.iter().find(|r| r.osm_ids[0].osm_way_id == from_osm),
            roads.iter().find(|r| r.osm_ids[0].osm_way_id == to_osm),
        ) {
            restrictions.push((from.id, restriction, to.id));
        }
    }
    for (from, rt, to) in restrictions {
        streets
            .roads
            .get_mut(&from)
            .unwrap()
            .turn_restrictions
            .push((rt, to));
    }

    // Resolve complicated turn restrictions (via a way). TODO Only handle via ways immediately
    // connected to both roads, for now
    let mut complicated_restrictions = Vec::new();
    for (rel_osm, from_osm, via_osm, to_osm) in input.complicated_turn_restrictions {
        let via_candidates: Vec<&Road> = streets
            .roads
            .values()
            .filter(|r| r.osm_ids[0].osm_way_id == via_osm)
            .collect();
        if via_candidates.len() != 1 {
            warn!(
                "Couldn't resolve turn restriction from way {} to way {} via way {}. Candidate \
                 roads for via: {:?}. See {}",
                from_osm, to_osm, via_osm, via_candidates, rel_osm
            );
            continue;
        }
        let via = via_candidates[0];

        let maybe_from = streets
            .roads_per_intersection(via.src_i)
            .into_iter()
            .chain(streets.roads_per_intersection(via.dst_i).into_iter())
            .find(|r| r.osm_ids[0].osm_way_id == from_osm);
        let maybe_to = streets
            .roads_per_intersection(via.src_i)
            .into_iter()
            .chain(streets.roads_per_intersection(via.dst_i).into_iter())
            .find(|r| r.osm_ids[0].osm_way_id == to_osm);
        match (maybe_from, maybe_to) {
            (Some(from), Some(to)) => {
                complicated_restrictions.push((from.id, via.id, to.id));
            }
            _ => {
                warn!(
                    "Couldn't resolve turn restriction from {} to {} via {:?}",
                    from_osm, to_osm, via
                );
            }
        }
    }
    for (from, via, to) in complicated_restrictions {
        streets
            .roads
            .get_mut(&from)
            .unwrap()
            .complicated_turn_restrictions
            .push((via, to));
    }

    timer.start("match traffic signals to intersections");
    // Handle traffic signals tagged on incoming ways and not at intersections
    // (https://wiki.openstreetmap.org/wiki/Tag:highway=traffic%20signals?uselang=en#Tag_all_incoming_ways).
    for (pt, dir) in input.traffic_signals {
        if let Some(r) = pt_to_road.get(&pt) {
            // The road might've crossed the boundary and been clipped
            if let Some(road) = streets.roads.get(r) {
                // Example: https://www.openstreetmap.org/node/26734224
                if road.highway_type != "construction" {
                    let i = if dir == Direction::Fwd {
                        road.dst_i
                    } else {
                        road.src_i
                    };
                    streets.intersections.get_mut(&i).unwrap().control =
                        IntersectionControl::Signalled;
                }
            }
        }
    }
    timer.stop("match traffic signals to intersections");

    timer.start("calculate intersection movements");
    let intersection_ids = osm_id_to_id.values();
    for &i in intersection_ids {
        streets.sort_roads(i);
        streets.update_movements(i);
    }
    timer.stop("calculate intersection movements");

    timer.stop("splitting up roads");
    pt_to_road
}

// TODO Consider doing this in PolyLine::new always. Also in extend() -- it attempts to dedupe
// angles.
fn simplify_linestring(pts: Vec<Pt2D>) -> Vec<Pt2D> {
    // Reduce the number of points along curves. They're wasteful, and when they're too close
    // together, actually break PolyLine shifting:
    // https://github.com/a-b-street/abstreet/issues/833
    //
    // The epsilon is in units of meters; points closer than this will get simplified. 0.1 is too
    // loose -- a curve with too many points was still broken, but 1.0 was too aggressive -- curves
    // got noticeably flattened. At 0.5, some intersetion polygons get a bit worse, but only in
    // places where they were already pretty broken.
    let epsilon = 0.5;
    Pt2D::simplify_rdp(pts, epsilon)
}

/// Many "roundabouts" like https://www.openstreetmap.org/way/427144965 are so tiny that they wind
/// up with ridiculous geometry, cause constant gridlock, and prevent merging adjacent blocks.
///
/// Note https://www.openstreetmap.org/way/394991047 is an example of something that shouldn't get
/// modified. The only distinction, currently, is length -- but I'd love a better definition.
/// Possibly the number of connecting roads.
fn should_collapse_roundabout(pts: &[Pt2D], tags: &Tags) -> bool {
    tags.is_any("junction", vec!["roundabout", "circular"])
        && pts[0] == *pts.last().unwrap()
        && PolyLine::unchecked_new(pts.to_vec()).length() < Distance::meters(50.0)
}
