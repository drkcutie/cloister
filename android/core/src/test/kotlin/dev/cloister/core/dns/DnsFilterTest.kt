package dev.cloister.core.dns

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The VPN's DNS filter at its seam: an IP packet read from the tun goes in, the packet to write
 * back comes out. Packets here are built by hand from RFC 791/8200/768/1035, not by the code.
 */
class DnsFilterTest {
    private val blocked = setOf("youtube.com", "youtu.be", "reddit.com")

    private val client4 = byteArrayOf(10, 111, 222.toByte(), 1)
    private val resolver4 = byteArrayOf(10, 111, 222.toByte(), 2)

    @Test
    fun aBlockedDomainGetsNxdomainWithoutAskingUpstream() {
        val filter = DnsFilter(blocked) { error("upstream must not be asked") }
        val reply = filter.handle(ipv4Query("youtube.com"))!!
        assertNxdomainFor(reply, "youtube.com")
    }

    @Test
    fun aSubdomainOfABlockedDomainIsBlocked() {
        val filter = DnsFilter(blocked) { error("upstream must not be asked") }
        assertNxdomainFor(filter.handle(ipv4Query("M.YouTube.com"))!!, "M.YouTube.com")
    }

    @Test
    fun aLookalikeDomainIsNotBlocked() {
        var asked: ByteArray? = null
        val answer = byteArrayOf(0x12, 0x34, 0x81.toByte(), 0x80.toByte(), 0, 1, 0, 1, 0, 0, 0, 0)
        val filter = DnsFilter(blocked) { asked = it; answer }
        val reply = filter.handle(ipv4Query("notyoutube.com"))!!
        assertEquals("notyoutube.com", DnsMessage.questionName(asked!!))
        // The upstream answer is passed back unchanged, addressed from the resolver to the client.
        assertArrayEquals(answer, udpPayload4(reply))
        assertEquals(53, port(reply, 20))
        assertEquals(40000, port(reply, 22))
    }

    @Test
    fun anUnreachableUpstreamGivesServfail() {
        val filter = DnsFilter(blocked) { null }
        val reply = filter.handle(ipv4Query("example.com"))!!
        assertEquals(2, udpPayload4(reply)[3].toInt() and 0x0f)
    }

    @Test
    fun ipv6QueriesAreFilteredToo() {
        val filter = DnsFilter(blocked) { error("upstream must not be asked") }
        val query = ipv6Query("redd.it.reddit.com")
        val reply = filter.handle(query)!!
        assertEquals(6, (reply[0].toInt() ushr 4))
        // Source and destination swapped.
        assertArrayEquals(query.copyOfRange(24, 40), reply.copyOfRange(8, 24))
        assertArrayEquals(query.copyOfRange(8, 24), reply.copyOfRange(24, 40))
        val dns = reply.copyOfRange(48, reply.size)
        assertEquals(3, dns[3].toInt() and 0x0f)
        assertEquals(0, onesComplement(pseudo6(reply) + reply.copyOfRange(40, reply.size)))
    }

    @Test
    fun nonDnsTrafficIsDropped() {
        val filter = DnsFilter(blocked) { error("no") }
        val tcp = ipv4Query("youtube.com").also { it[9] = 6 }
        assertNull(filter.handle(tcp))
        assertNull(filter.handle(byteArrayOf(0x45, 0)))
    }

    // --- hand-built packets and checks ---

    private fun dnsQuery(name: String): ByteArray {
        val header = byteArrayOf(0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0)
        val qname = name.split('.').flatMap { listOf(it.length.toByte()) + it.toByteArray().toList() } + 0.toByte()
        return header + qname.toByteArray() + byteArrayOf(0, 1, 0, 1)
    }

    private fun ipv4Query(name: String): ByteArray {
        val dns = dnsQuery(name)
        val udpLen = 8 + dns.size
        val total = 20 + udpLen
        val ip = byteArrayOf(
            0x45, 0, (total ushr 8).toByte(), total.toByte(), 0, 1, 0, 0, 64, 17, 0, 0,
        ) + client4 + resolver4
        val udp = byteArrayOf(0x9c.toByte(), 0x40, 0, 53, (udpLen ushr 8).toByte(), udpLen.toByte(), 0, 0)
        return ip + udp + dns
    }

    private fun ipv6Query(name: String): ByteArray {
        val dns = dnsQuery(name)
        val udpLen = 8 + dns.size
        val src = ByteArray(16).also { it[0] = 0xfd.toByte(); it[15] = 1 }
        val dst = ByteArray(16).also { it[0] = 0xfd.toByte(); it[15] = 2 }
        val ip = byteArrayOf(0x60, 0, 0, 0, (udpLen ushr 8).toByte(), udpLen.toByte(), 17, 64) + src + dst
        return ip + byteArrayOf(0x9c.toByte(), 0x40, 0, 53, (udpLen ushr 8).toByte(), udpLen.toByte(), 0, 0) + dns
    }

    private fun assertNxdomainFor(reply: ByteArray, name: String) {
        assertEquals(4, reply[0].toInt() ushr 4)
        assertArrayEquals(resolver4, reply.copyOfRange(12, 16))
        assertArrayEquals(client4, reply.copyOfRange(16, 20))
        assertEquals(reply.size, u16(reply, 2))
        assertEquals("IPv4 header checksum", 0, onesComplement(reply.copyOfRange(0, 20)))
        assertEquals("UDP checksum", 0, onesComplement(pseudo4(reply) + reply.copyOfRange(20, reply.size)))
        val dns = udpPayload4(reply)
        assertEquals(0x1234, u16(dns, 0))
        assertEquals("QR set", 0x80, dns[2].toInt() and 0x80)
        assertEquals("NXDOMAIN", 3, dns[3].toInt() and 0x0f)
        assertEquals("one question echoed", 1, u16(dns, 4))
        assertEquals(name, DnsMessage.questionName(dns))
    }

    private fun udpPayload4(p: ByteArray) = p.copyOfRange(28, p.size)
    private fun port(p: ByteArray, at: Int) = u16(p, at)
    private fun u16(b: ByteArray, at: Int) = ((b[at].toInt() and 0xff) shl 8) or (b[at + 1].toInt() and 0xff)

    private fun pseudo4(p: ByteArray): ByteArray {
        val len = p.size - 20
        return p.copyOfRange(12, 20) + byteArrayOf(0, 17, (len ushr 8).toByte(), len.toByte())
    }

    private fun pseudo6(p: ByteArray): ByteArray {
        val len = p.size - 40
        return p.copyOfRange(8, 40) + byteArrayOf(0, 0, (len ushr 8).toByte(), len.toByte(), 0, 0, 0, 17)
    }

    /** One's-complement sum folded and inverted: 0 when the embedded checksum is right. */
    private fun onesComplement(b: ByteArray): Int {
        var sum = 0L
        var i = 0
        while (i < b.size) {
            val hi = b[i].toInt() and 0xff
            val lo = if (i + 1 < b.size) b[i + 1].toInt() and 0xff else 0
            sum += (hi shl 8) or lo
            i += 2
        }
        while (sum ushr 16 != 0L) sum = (sum and 0xffff) + (sum ushr 16)
        return (sum.inv() and 0xffff).toInt()
    }
}
