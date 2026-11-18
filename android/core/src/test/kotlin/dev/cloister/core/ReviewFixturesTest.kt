package dev.cloister.core

import java.io.File
import java.time.Instant
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertTrue
import org.junit.Test

/** The scoreboard ([Review.weekSummary]) against the shared scenarios in `fixtures/review`, as the Rust engine runs them. */
class ReviewFixturesTest {
    @Test
    fun everyReviewScenarioPasses() {
        val dir = File(System.getProperty("cloister.reviewFixtures") ?: "../../fixtures/review")
        val files = dir.listFiles { f -> f.extension == "json" }.orEmpty().sortedBy { it.name }
        assertTrue("no review fixtures found in $dir", files.isNotEmpty())
        val failures = files.mapNotNull { f ->
            val scenario = Json.parseToJsonElement(f.readText()).jsonObject
            val unknown = scenario.keys - setOf("name", "description", "state", "at", "expect")
            if (unknown.isNotEmpty()) return@mapNotNull "${f.name}: unknown keys $unknown"
            val state = State.fromJson(scenario.getValue("state"))
            val at = Instant.parse(scenario.getValue("at").jsonPrimitive.content)
            val got = Review.weekSummary(state, at).toJson()
            val want = scenario.getValue("expect")
            if (got != want) "${f.name}:\n want $want\n  got $got" else null
        }
        assertTrue(failures.joinToString("\n\n"), failures.isEmpty())
    }
}
