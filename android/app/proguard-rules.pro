# kotlinx.serialization: keep the generated serializers of the protocol classes.
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.**
-keepclassmembers class app.pill.phone.protocol.** {
    *** Companion;
}
-keepclasseswithmembers class app.pill.phone.protocol.** {
    kotlinx.serialization.KSerializer serializer(...);
}
# androidx.security's Tink dependency references compile-time-only annotations.
-dontwarn com.google.errorprone.annotations.**
-dontwarn javax.annotation.**
