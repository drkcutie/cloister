package dev.cloister.core

import java.io.File
import java.time.Instant
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Seam A: runs every shared JSON scenario in `fixtures/rules` against the Kotlin rules engine,
 * following the runner semantics in `fixtures/README.md`. The directory comes from the
 * `cloister.fixtures` system property set by Gradle.
 */
class SharedFixturesTest {
    @Test
    fun everySharedFixturePasses() {
        val dir = File(System.getProperty("cloister.fixtures") ?: "../../fixtures/rules")
        val files = dir.listFiles { f -> f.extension == "json" }.orEmpty().sortedBy { it.name }
        assertTrue("no fixtures found in $dir", files.isNotEmpty())
        val failures = files.mapNotNull { f ->
            try {
                run(f)
                null
            } catch (e: Throwable) {
                "${f.name}: ${e.message}"
            }
        }
        assertTrue(
            "${failures.size} of ${files.size} fixture(s) failed:\n" + failures.joinToString("\n\n"),
            failures.isEmpty(),
        )
    }

    private fun run(file: File) {
        val scenario = Json.parseToJsonElement(file.readText()).jsonObject
        scenario.only("name", "description", "initial_state", "steps")
        val name = scenario.getValue("name").jsonPrimitive.content
        var state = State.fromJson(scenario.getValue("initial_state"))
        for ((i, stepEl) in scenario.getValue("steps").jsonArray.withIndex()) {
            fun fail(msg: String): Nothing = throw AssertionError("$name step $i: $msg")
            val step = stepEl.jsonObject
            step.only("server_time", "command", "expect")
            val now = parseTime(step.getValue("server_time").jsonPrimitive.content)
            val expect = step.getValue("expect").jsonObject
            expect.only("outcome", "reason", "state", "enforcement")
            val command = step["command"]?.takeUnless { it is JsonNull }
            val current = if (command != null) {
                val decision = RulesEngine.decide(state, Command.fromJson(command), now)
                val (outcome, reason) = when (val o = decision.outcome) {
                    Outcome.Accepted -> "accepted" to null
                    is Outcome.Rejected -> "rejected" to o.reason.code
                }
                val want = expect["outcome"]?.jsonPrimitive?.content
                    ?: fail("a step with a command must expect an outcome")
                val wantReason = expect["reason"]?.takeUnless { it is JsonNull }?.jsonPrimitive?.content
                if (outcome != want || reason != wantReason) {
                    fail("expected $want $wantReason, got $outcome $reason")
                }
                if (outcome == "rejected" && decision.state != state) {
                    fail("a rejected command changed the state")
                }
                state = decision.state
                decision.enforcement
            } else {
                RulesEngine.enforcement(state, now)
            }
            expect["state"]?.let { want ->
                val got = state.toJson()
                if (got != want) fail("state mismatch\n want $want\n  got $got")
            }
            expect["enforcement"]?.let { checkEnforcement(current, it.jsonObject)?.let(::fail) }
        }
    }

    private fun checkEnforcement(got: Enforcement, want: JsonObject): String? {
        want.only("blocked_domains", "blocked_domains_include", "blocked_domains_exclude", "blocked_apps", "ritual")
        val domains = got.blockedDomains
        want["blocked_domains"]?.strings()?.sorted()?.let {
            if (it != domains) return "blocked_domains: want $it, got $domains"
        }
        want["blocked_domains_include"]?.strings()?.forEach {
            if (it !in domains) return "$it should be blocked, got $domains"
        }
        want["blocked_domains_exclude"]?.strings()?.forEach {
            if (it in domains) return "$it should not be blocked"
        }
        want["blocked_apps"]?.strings()?.sorted()?.let {
            if (it != got.blockedApps) return "blocked_apps: want $it, got ${got.blockedApps}"
        }
        want["ritual"]?.jsonPrimitive?.content?.let {
            if (it != got.ritual.code) return "ritual: want $it, got ${got.ritual.code}"
        }
        return null
    }

    private fun JsonElement.strings() = jsonArray.map { (it as JsonPrimitive).content }

    private fun JsonObject.only(vararg keys: String) {
        val unknown = this.keys - keys.toSet()
        if (unknown.isNotEmpty()) throw AssertionError("unknown keys $unknown")
    }

    private fun parseTime(s: String): Instant = java.time.OffsetDateTime.parse(s).toInstant()
}
