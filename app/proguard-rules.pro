# JNI entry points are looked up by name from Rust
-keep class org.rynk.keyboard.Native { *; }
-keepclasseswithmembernames class * { native <methods>; }
