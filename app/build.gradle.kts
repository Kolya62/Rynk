// SPDX-License-Identifier: GPL-3.0-or-later
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

val localProps = Properties().apply {
    val f = rootProject.file("local.properties")
    if (f.exists()) f.inputStream().use { load(it) }
}
fun secret(env: String, prop: String): String? = System.getenv(env) ?: localProps.getProperty(prop)

android {
    namespace = "org.rynk.keyboard"
    compileSdk = 35
    ndkVersion = "26.3.11579264"

    defaultConfig {
        applicationId = "org.rynk.keyboard"
        minSdk = 26
        targetSdk = 35
        versionCode = 6
        versionName = "0.2.2"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }

    signingConfigs {
        create("release") {
            // keys never live in the repository: environment variables or local.properties
            val store = secret("RYNK_KEYSTORE_PATH", "rynk.release.storeFile")
            if (store != null && file(store).exists()) {
                storeFile = file(store)
                storePassword = secret("RYNK_KEYSTORE_PASSWORD", "rynk.release.storePassword")
                keyAlias = secret("RYNK_KEY_ALIAS", "rynk.release.keyAlias")
                keyPassword = secret("RYNK_KEY_PASSWORD", "rynk.release.keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.getByName("release").takeIf { it.storeFile != null }
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("rustJniLibs"))

    // compressed native libraries and assets: smaller APK (libraries are extracted on install)
    packaging { jniLibs { useLegacyPackaging = true } }
    // models and dictionaries are already zstd-compressed; deflating them again gains nothing
    androidResources { noCompress += listOf("rlm", "rcd") }
}

kotlin {
    compilerOptions { jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17) }
}

/** Builds the Rust core for every ABI with cargo-ndk into build/rustJniLibs */
val buildRust = tasks.register<Exec>("buildRust") {
    group = "build"
    description = "Compiles rust-core (cargo ndk)"
    workingDir = rootDir
    val out = layout.buildDirectory.dir("rustJniLibs").get().asFile
    inputs.dir(rootProject.file("rust-core/src"))
    inputs.file(rootProject.file("rust-core/Cargo.toml"))
    outputs.dir(out)
    val sdk = localProps.getProperty("sdk.dir") ?: System.getenv("ANDROID_HOME") ?: "${System.getProperty("user.home")}/Android/Sdk"
    environment("ANDROID_NDK_HOME", System.getenv("ANDROID_NDK_HOME") ?: "$sdk/ndk/26.3.11579264")
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a", "-t", "armeabi-v7a", "-t", "x86_64",
        "--platform", "26",
        "-o", out.absolutePath,
        "build", "--release", "-p", "rynk_core",
    )
}

tasks.named("preBuild") { dependsOn(buildRust) }
