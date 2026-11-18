// Sync with the Supabase backend (supabase-kt): sign-in, Device registration, commands, the
// cached state (mirror) and Realtime. Plain Kotlin on the JVM so it can be tested against a
// local Supabase without an emulator; the Android app uses it as is.
plugins {
    id("org.jetbrains.kotlin.jvm")
}

kotlin { jvmToolchain(17) }

dependencies {
    api(project(":core"))
    api(platform("io.github.jan-tennert.supabase:bom:3.9.0"))
    api("io.github.jan-tennert.supabase:auth-kt")
    api("io.github.jan-tennert.supabase:postgrest-kt")
    api("io.github.jan-tennert.supabase:realtime-kt")
    api("io.ktor:ktor-client-okhttp:3.6.1")
    api("org.jetbrains.kotlinx:kotlinx-serialization-json:1.9.0")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.10.2")
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.10.2")
}

tasks.test {
    // The backend tests need a local Supabase (backend/scripts/test.sh exports these).
    listOf("CLOISTER_API_URL", "CLOISTER_ANON_KEY", "CLOISTER_MAILPIT_URL").forEach { key ->
        System.getenv(key)?.let { environment(key, it) }
    }
    testLogging { events("failed", "skipped"); exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL }
}
