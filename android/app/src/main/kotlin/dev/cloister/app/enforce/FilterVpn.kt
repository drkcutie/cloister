package dev.cloister.app.enforce

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log
import dev.cloister.app.CloisterApp
import dev.cloister.app.MainActivity
import dev.cloister.app.R
import dev.cloister.core.dns.DnsFilter
import java.io.FileInputStream
import java.io.FileOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.concurrent.Executors

/**
 * The DNS filter (ADR 0002 on the phone). The tun carries only the fake resolver's address, so
 * apps' DNS queries arrive here and nothing else does: no packet forwarding is needed. A
 * blocked domain or any subdomain gets NXDOMAIN; everything else is asked of the real
 * resolver over a socket that bypasses the VPN.
 */
class FilterVpn : VpnService() {
    private var tun: ParcelFileDescriptor? = null
    private var reader: Thread? = null
    private val workers = Executors.newFixedThreadPool(4)

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startInForeground()
        if (tun == null) establish()
        return START_STICKY
    }

    private fun establish() {
        val fd = Builder()
            .setSession("Cloister")
            .addAddress(CLIENT_V4, 32)
            .addDnsServer(RESOLVER_V4)
            .addRoute(RESOLVER_V4, 32)
            .addAddress(CLIENT_V6, 128)
            .addDnsServer(RESOLVER_V6)
            .addRoute(RESOLVER_V6, 128)
            .setBlocking(true)
            .setConfigureIntent(
                PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
            )
            .establish()
        if (fd == null) {
            // Not prepared (consent missing or taken by another VPN).
            stopSelf()
            return
        }
        tun = fd
        running = true
        CloisterApp.instance.enforcer.vpnWanted = true
        reader = Thread({ loop(fd) }, "cloister-dns").also { it.start() }
        CloisterApp.instance.enforcer.tick()
    }

    private fun loop(fd: ParcelFileDescriptor) {
        val input = FileInputStream(fd.fileDescriptor)
        val output = FileOutputStream(fd.fileDescriptor)
        val buf = ByteArray(32 * 1024)
        try {
            while (!Thread.interrupted()) {
                val n = input.read(buf)
                if (n <= 0) continue
                val packet = buf.copyOf(n)
                workers.execute {
                    val reply = runCatching { filter.handle(packet) }.getOrNull() ?: return@execute
                    synchronized(output) { runCatching { output.write(reply) } }
                }
            }
        } catch (e: Exception) {
            Log.i(TAG, "DNS loop ended: $e")
        }
    }

    /** Asks the network's real resolver, outside the VPN. */
    private fun upstream(query: ByteArray): ByteArray? {
        for (server in upstreamServers()) {
            try {
                DatagramSocket().use { socket ->
                    protect(socket)
                    socket.soTimeout = 4_000
                    socket.send(DatagramPacket(query, query.size, InetSocketAddress(server, 53)))
                    val buf = ByteArray(4096)
                    val answer = DatagramPacket(buf, buf.size)
                    socket.receive(answer)
                    return buf.copyOf(answer.length)
                }
            } catch (e: Exception) {
                Log.d(TAG, "upstream $server: $e")
            }
        }
        return null
    }

    private fun upstreamServers(): List<InetAddress> {
        val cm = getSystemService(ConnectivityManager::class.java)
        @Suppress("DEPRECATION")
        val fromNetworks = cm.allNetworks.filter { n ->
            val caps = cm.getNetworkCapabilities(n) ?: return@filter false
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) &&
                caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        }.flatMap { cm.getLinkProperties(it)?.dnsServers.orEmpty() }
        return (fromNetworks + FALLBACK).distinct()
    }

    override fun onRevoke() {
        Log.i(TAG, "VPN revoked")
        shutdown()
        CloisterApp.instance.enforcer.vpnRevoked()
        super.onRevoke()
    }

    override fun onDestroy() {
        shutdown()
        super.onDestroy()
    }

    private fun shutdown() {
        running = false
        reader?.interrupt()
        runCatching { tun?.close() }
        tun = null
        if (instance === this) instance = null
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    private fun startInForeground() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Enforcement", NotificationManager.IMPORTANCE_MIN))
        val notification = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle("Cloister is enforcing")
            .setContentText("Blocked sites don't load during a Block.")
            .setOngoing(true)
            .setContentIntent(
                PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE),
            )
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(1, notification)
        }
    }

    companion object {
        private const val TAG = "CloisterVpn"
        private const val CHANNEL = "enforcement"
        const val CLIENT_V4 = "10.111.222.1"
        const val RESOLVER_V4 = "10.111.222.2"
        const val CLIENT_V6 = "fd00:111:222::1"
        const val RESOLVER_V6 = "fd00:111:222::2"
        private val FALLBACK = listOf(InetAddress.getByName("1.1.1.1"), InetAddress.getByName("8.8.8.8"))

        @Volatile var running = false
            private set
        @Volatile private var instance: FilterVpn? = null
        @Volatile private var blocked: List<String> = emptyList()
        @Volatile private var filter = DnsFilter(emptyList()) { q -> instance?.upstream(q) }

        /** Sets the blocked domains (each also blocks its subdomains). */
        fun update(domains: List<String>) {
            if (domains == blocked) return
            blocked = domains
            filter = DnsFilter(domains) { q -> instance?.upstream(q) }
        }
    }
}
