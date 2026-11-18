package dev.cloister.core.dns

/**
 * The DNS filter behind Android's VPN: only the fake resolver address is routed into the tun,
 * so every packet read from it is a DNS query. [handle] takes that IP packet and returns the
 * packet to write back: NXDOMAIN for a blocked name (or any subdomain of one), otherwise the
 * answer from [upstream] (a socket that bypasses the VPN), or SERVFAIL when upstream fails.
 * Packets that aren't UDP DNS queries return null and are dropped.
 */
class DnsFilter(blocked: Collection<String>, private val upstream: (ByteArray) -> ByteArray?) {
    private val matcher = DomainMatcher(blocked)

    /** Whether a lookup of [name] is refused. */
    fun isBlocked(name: String) = matcher.isBlocked(name)

    fun handle(packet: ByteArray): ByteArray? {
        val udp = UdpPacket.parse(packet) ?: return null
        if (udp.dstPort != 53) return null
        val query = udp.payload
        val name = DnsMessage.questionName(query) ?: return null
        val answer = if (matcher.isBlocked(name)) {
            DnsMessage.reply(query, DnsMessage.NXDOMAIN)
        } else {
            upstream(query) ?: DnsMessage.reply(query, DnsMessage.SERVFAIL)
        }
        return udp.reply(answer)
    }
}

/** A blocked domain covers itself and all of its subdomains; names compare case-insensitively. */
class DomainMatcher(blocked: Collection<String>) {
    private val set = blocked.map { it.lowercase().removeSuffix(".") }.toHashSet()

    fun isBlocked(name: String): Boolean {
        var d = name.lowercase().removeSuffix(".")
        while (true) {
            if (d in set) return true
            val dot = d.indexOf('.')
            if (dot < 0) return false
            d = d.substring(dot + 1)
        }
    }
}

/** Just enough of RFC 1035 to read the question and answer with an error code. */
object DnsMessage {
    const val SERVFAIL = 2
    const val NXDOMAIN = 3

    /** The first question's name, dotted, or null if [msg] isn't a well-formed standard query. */
    fun questionName(msg: ByteArray): String? {
        if (msg.size < 12 || u16(msg, 4) < 1) return null
        val end = questionEnd(msg) ?: return null
        val labels = mutableListOf<String>()
        var i = 12
        while (msg[i].toInt() != 0) {
            val len = msg[i].toInt() and 0xff
            labels += String(msg, i + 1, len, Charsets.US_ASCII)
            i += len + 1
        }
        return if (end <= msg.size) labels.joinToString(".") else null
    }

    /** Offset just past the first question (name, type, class), or null. Compression is not allowed there. */
    private fun questionEnd(msg: ByteArray): Int? {
        var i = 12
        while (true) {
            if (i >= msg.size) return null
            val len = msg[i].toInt() and 0xff
            if (len == 0) break
            if (len and 0xc0 != 0) return null
            i += len + 1
        }
        val end = i + 1 + 4
        return if (end <= msg.size) end else null
    }

    /** A response to [query] carrying only its first question and [rcode]. */
    fun reply(query: ByteArray, rcode: Int): ByteArray {
        val end = questionEnd(query) ?: 12
        val out = query.copyOfRange(0, end)
        out[2] = ((query[2].toInt() and 0x01) or 0x80).toByte() // QR, keep RD
        out[3] = (0x80 or rcode).toByte() // RA + rcode
        out[4] = 0; out[5] = if (end > 12) 1 else 0
        for (k in 6 until 12) out[k] = 0
        return out
    }
}

/** An IPv4 or IPv6 UDP packet (no IPv6 extension headers). */
class UdpPacket private constructor(
    private val version: Int,
    private val src: ByteArray,
    private val dst: ByteArray,
    val srcPort: Int,
    val dstPort: Int,
    val payload: ByteArray,
) {
    /** A packet from this one's destination back to its source carrying [payload]. */
    fun reply(payload: ByteArray): ByteArray {
        val udpLen = 8 + payload.size
        val udp = ByteArray(udpLen)
        put16(udp, 0, dstPort); put16(udp, 2, srcPort); put16(udp, 4, udpLen)
        payload.copyInto(udp, 8)
        return if (version == 4) {
            val ip = ByteArray(20)
            ip[0] = 0x45; put16(ip, 2, 20 + udpLen); ip[8] = 64; ip[9] = 17
            dst.copyInto(ip, 12); src.copyInto(ip, 16)
            put16(ip, 10, checksum(ip))
            val pseudo = dst + src + byteArrayOf(0, 17, (udpLen ushr 8).toByte(), udpLen.toByte())
            put16(udp, 6, checksum(pseudo + udp).let { if (it == 0) 0xffff else it })
            ip + udp
        } else {
            val ip = ByteArray(40)
            ip[0] = 0x60; put16(ip, 4, udpLen); ip[6] = 17; ip[7] = 64
            dst.copyInto(ip, 8); src.copyInto(ip, 24)
            val pseudo = dst + src + byteArrayOf(0, 0, (udpLen ushr 8).toByte(), udpLen.toByte(), 0, 0, 0, 17)
            put16(udp, 6, checksum(pseudo + udp).let { if (it == 0) 0xffff else it })
            ip + udp
        }
    }

    companion object {
        fun parse(p: ByteArray): UdpPacket? {
            if (p.isEmpty()) return null
            return when (p[0].toInt() ushr 4) {
                4 -> {
                    if (p.size < 28) return null
                    val ihl = (p[0].toInt() and 0x0f) * 4
                    if (p[9].toInt() != 17 || ihl < 20 || p.size < ihl + 8) return null
                    val total = minOf(u16(p, 2), p.size)
                    udp(4, p.copyOfRange(12, 16), p.copyOfRange(16, 20), p, ihl, total)
                }
                6 -> {
                    if (p.size < 48 || p[6].toInt() != 17) return null
                    val total = minOf(40 + u16(p, 4), p.size)
                    udp(6, p.copyOfRange(8, 24), p.copyOfRange(24, 40), p, 40, total)
                }
                else -> null
            }
        }

        private fun udp(v: Int, src: ByteArray, dst: ByteArray, p: ByteArray, at: Int, total: Int): UdpPacket? {
            val len = u16(p, at + 4)
            if (len < 8 || at + len > total) return null
            return UdpPacket(v, src, dst, u16(p, at), u16(p, at + 2), p.copyOfRange(at + 8, at + len))
        }
    }
}

private fun u16(b: ByteArray, at: Int) = ((b[at].toInt() and 0xff) shl 8) or (b[at + 1].toInt() and 0xff)

private fun put16(b: ByteArray, at: Int, v: Int) {
    b[at] = (v ushr 8).toByte(); b[at + 1] = v.toByte()
}

/** The Internet checksum (RFC 1071) of [b]. */
private fun checksum(b: ByteArray): Int {
    var sum = 0L
    var i = 0
    while (i < b.size) {
        sum += ((b[i].toInt() and 0xff) shl 8) or (if (i + 1 < b.size) b[i + 1].toInt() and 0xff else 0)
        i += 2
    }
    while (sum ushr 16 != 0L) sum = (sum and 0xffff) + (sum ushr 16)
    return (sum.inv() and 0xffff).toInt()
}
