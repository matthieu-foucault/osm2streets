/* DO NOT EDIT THIS FILE - it is machine generated */
#include <jni.h>
/* Header for class StreetNetwork */

#ifndef _Included_StreetNetwork
#define _Included_StreetNetwork
#ifdef __cplusplus
extern "C" {
#endif
/*
 * Class:     StreetNetwork
 * Method:    create
 * Signature: (Ljava/lang/String;)LStreetNetwork;
 */
JNIEXPORT jobject JNICALL Java_StreetNetwork_create
  (JNIEnv *, jclass, jstring);

/*
 * Class:     StreetNetwork
 * Method:    toGeojsonPlain
 * Signature: ()Ljava/lang/String;
 */
JNIEXPORT jstring JNICALL Java_StreetNetwork_toGeojsonPlain
  (JNIEnv *, jobject);

/*
 * Class:     StreetNetwork
 * Method:    toLanePolygonsGeojson
 * Signature: ()Ljava/lang/String;
 */
JNIEXPORT jstring JNICALL Java_StreetNetwork_toLanePolygonsGeojson
  (JNIEnv *, jobject);

/*
 * Class:     StreetNetwork
 * Method:    toLaneMarkingsGeojson
 * Signature: ()Ljava/lang/String;
 */
JNIEXPORT jstring JNICALL Java_StreetNetwork_toLaneMarkingsGeojson
  (JNIEnv *, jobject);

#ifdef __cplusplus
}
#endif
#endif
