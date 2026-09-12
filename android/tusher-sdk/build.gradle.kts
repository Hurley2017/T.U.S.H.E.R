plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "com.tusher.android"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        version = 1
    }

    lint {
        targetSdk = 35
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }

    // JNI libraries produced by cargo-ndk go here
    sourceSets["main"].jniLibs.srcDirs("src/main/jniLibs")

    // Exclude source files that reference the UniFFI-generated bindings
    // (tusher_ffi.kt is 2500+ lines and has compile-time circular deps when
    //  included alongside the hand-written SDK layer in the same module).
    // The generated bindings are compiled as part of the uniffi source set.
    sourceSets["main"].java.exclude(
        "com/tusher/android/TusherService.kt"
    )
}

dependencies {
    implementation(libs.kotlin.stdlib)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.documentfile)
    implementation(libs.jna)
    implementation(libs.jna.platform)
}

