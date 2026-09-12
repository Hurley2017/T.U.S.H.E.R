package com.tusher.android

import android.app.*
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Binder
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import uniffi.tusher_ffi.*

/**
 * Android Foreground Service hosting the T.U.S.H.E.R peer-to-peer node daemon.
 * Ensures the Rust Tokio mesh engine continues to synchronize files, answer discovery beacons,
 * and relay transitive data when the screen is off or the app is in the background.
 */
class TusherService : Service(), FfiEventListener {

    private val binder = LocalBinder()
    private var node: TusherNode? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null

    inner class LocalBinder : Binder() {
        fun getService(): TusherService = this@TusherService
    }

    override fun onBind(intent: Intent?): IBinder = binder

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        acquireWakeLocks()
    }

    /**
     * Initializes and starts the embedded Rust mesh node within the foreground service.
     */
    fun startMeshNode(
        dataDir: String,
        deviceName: String,
        port: UShort = 42931u,
        discoveryPort: UShort = 42831u
    ): TusherNode {
        if (node == null) {
            val tusherNode = TusherNode(dataDir, deviceName, port, discoveryPort)
            tusherNode.registerEventListener(this)
            node = tusherNode

            val notification = buildForegroundNotification("T.U.S.H.E.R Mesh Active", "Connected as $deviceName")
            startForeground(NOTIFICATION_ID, notification)
        }
        return node!!
    }

    fun getNode(): TusherNode? = node

    private fun acquireWakeLocks() {
        val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = powerManager.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "tusher:mesh_wakelock").apply {
            setReferenceCounted(false)
            acquire(24 * 60 * 60 * 1000L) // 24 hours max safeguard
        }

        val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        wifiLock = wifiManager.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "tusher:wifi_lock").apply {
            setReferenceCounted(false)
            acquire()
        }
    }

    private fun releaseWakeLocks() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wifiLock?.let { if (it.isHeld) it.release() }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "T.U.S.H.E.R Mesh Service",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Keeps the decentralized file mesh active in the background"
            }
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(channel)
        }
    }

    private fun buildForegroundNotification(title: String, content: String): Notification {
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(title)
            .setContentText(content)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    // --- FfiEventListener Callbacks ---

    override fun onPeerStatusChanged(peerId: String, peerName: String, isConnected: Boolean) {
        val statusText = if (isConnected) "Connected to $peerName" else "Disconnected from $peerName"
        val notification = buildForegroundNotification("T.U.S.H.E.R Mesh", statusText)
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    override fun onSyncEvent(folderId: String, relativePath: String, eventType: String) {
        val notification = buildForegroundNotification("T.U.S.H.E.R Synchronizing", "$eventType: $relativePath")
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    override fun onTransferCompleted(
        folderId: String,
        fileName: String,
        fileSize: ULong,
        contentHash: String
    ) {
        val notification = buildForegroundNotification(
            "Transfer Complete",
            "Received $fileName (${fileSize / 1024u} KB)"
        )
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    override fun onDestroy() {
        node?.close()
        node = null
        releaseWakeLocks()
        super.onDestroy()
    }

    companion object {
        const val CHANNEL_ID = "tusher_mesh_service_channel"
        const val NOTIFICATION_ID = 1001
    }
}
