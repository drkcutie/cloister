plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

// Backend location. Defaults reach the local Supabase from the emulator; pass
// -Pcloister.apiUrl=... -Pcloister.anonKey=... (or put them in ~/.gradle/gradle.properties).
fun prop(name: String, default: String) = providers.gradleProperty(name).getOrElse(default)

// The OFL fonts live once in the repo (desktop assets); they are copied in as font resources.
abstract class CopyFonts : DefaultTask() {
    @get:InputDirectory abstract val source: DirectoryProperty
    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction
    fun copy() {
        val fontDir = output.get().asFile.resolve("font").apply { deleteRecursively(); mkdirs() }
        source.get().asFile.listFiles { f -> f.extension == "ttf" }.orEmpty().forEach { f ->
            val name = f.nameWithoutExtension.replace(Regex("([a-z0-9])([A-Z])"), "$1_$2").replace('-', '_').lowercase()
            f.copyTo(fontDir.resolve("$name.ttf"), overwrite = true)
        }
    }
}
val copyFonts = tasks.register<CopyFonts>("copyFonts") {
    source.set(rootDir.resolve("../desktop/crates/cloister-app/assets/fonts"))
}
androidComponents {
    onVariants { variant -> variant.sources.res?.addGeneratedSourceDirectory(copyFonts, CopyFonts::output) }
}

android {
    namespace = "dev.cloister.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.cloister"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        buildConfigField("String", "API_URL", "\"${prop("cloister.apiUrl", "http://10.0.2.2:56421")}\"")
        buildConfigField("String", "ANON_KEY", "\"${prop("cloister.anonKey", "")}\"")
        buildConfigField("String", "GOOGLE_SERVER_CLIENT_ID", "\"${prop("cloister.googleServerClientId", "")}\"")
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    packaging {
        resources.excludes += setOf("META-INF/INDEX.LIST", "META-INF/*.kotlin_module", "META-INF/io.netty.versions.properties")
    }
}

dependencies {
    implementation(project(":sync"))
    implementation(platform("androidx.compose:compose-bom:2025.06.01"))
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.9.1")
    implementation("androidx.lifecycle:lifecycle-service:2.9.1")
    implementation("androidx.savedstate:savedstate-ktx:1.3.0")
    implementation("androidx.credentials:credentials:1.5.0")
    implementation("androidx.credentials:credentials-play-services-auth:1.5.0")
    implementation("com.google.android.libraries.identity.googleid:googleid:1.1.1")
    testImplementation("junit:junit:4.13.2")
}
