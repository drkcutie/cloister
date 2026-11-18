// The rules engine and the DNS filter logic: plain Kotlin on the JVM, no Android classes,
// so `./gradlew :core:test` runs the shared fixtures in seconds.
plugins {
    id("org.jetbrains.kotlin.jvm")
}

kotlin { jvmToolchain(17) }

// The built-in Blocklists ship inside the jar as the classpath resource `/blocklists.json`.
sourceSets.main { resources.srcDir(rootDir.resolve("../shared")) }

dependencies {
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    testImplementation("junit:junit:4.13.2")
}

tasks.test {
    // Seam A: the shared scenarios, read in place (never copied).
    systemProperty("cloister.fixtures", rootDir.resolve("../fixtures/rules").absolutePath)
    inputs.dir(rootDir.resolve("../fixtures/rules"))
    testLogging { events("failed"); exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL }
}
