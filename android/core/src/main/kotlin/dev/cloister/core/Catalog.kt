package dev.cloister.core

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

enum class BlocklistKind { Distraction, Work }

data class Blocklist(val id: String, val kind: BlocklistKind, val domains: List<String>, val apps: List<String>)

/** Built-in Blocklists and the related-domain map, from `shared/blocklists.json` (bundled as a resource). */
class Catalog(val blocklists: List<Blocklist>, val relatedDomains: Map<String, List<String>>) {
    fun get(id: String) = blocklists.firstOrNull { it.id == id }

    /** Ids of the Blocklists that make up the Distraction List, in catalog order. */
    fun distractionIds() = blocklists.filter { it.kind == BlocklistKind.Distraction }.map { it.id }

    /** Ids of the Work List's Blocklists. */
    fun workIds() = blocklists.filter { it.kind == BlocklistKind.Work }.map { it.id }

    /** Domains blocked together with [domain] (e.g. `youtube.com` -> `youtu.be`). */
    fun related(domain: String) = relatedDomains[domain].orEmpty()

    companion object {
        val builtin: Catalog by lazy {
            val text = Catalog::class.java.getResourceAsStream("/blocklists.json")
                ?.use { it.readBytes().decodeToString() }
                ?: error("blocklists.json is missing from the classpath")
            parse(text)
        }

        fun parse(text: String): Catalog {
            val root = Json.parseToJsonElement(text).jsonObject
            fun JsonArray.strings() = map { (it as JsonPrimitive).content }
            val lists = root.getValue("blocklists").jsonArray.map {
                val o = it.jsonObject
                Blocklist(
                    id = o.getValue("id").jsonPrimitive.content,
                    kind = when (val k = o.getValue("kind").jsonPrimitive.content) {
                        "distraction" -> BlocklistKind.Distraction
                        "work" -> BlocklistKind.Work
                        else -> error("unknown Blocklist kind $k")
                    },
                    domains = o.getValue("domains").jsonArray.strings(),
                    apps = o.getValue("apps").jsonArray.strings(),
                )
            }
            val related = (root["related_domains"] as? JsonObject).orEmpty()
                .mapValues { (_, v) -> v.jsonArray.strings() }
            return Catalog(lists, related)
        }
    }
}
