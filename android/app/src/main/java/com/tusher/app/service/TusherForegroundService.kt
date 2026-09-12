package com.tusher.app.service

import android.app.Notification
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Binder
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.tusher.app.R
import com.tusher.app.ui.MainActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Foreground service that hosts the T.U.S.H.E.R sync node.
 * Keeps the Rust engine alive while the app is in the background.
 */
class TusherForegroundService : Service() {

    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var syncJob: Job? = null

    private val _syncState = MutableStateFlow(SyncState())
    val syncState: StateFlow<SyncState> = _syncState

    inner class LocalBinder : Binder() {
        fun getService(): TusherForegroundService = this@TusherForegroundService
    }

    private val binder = LocalBinder()

    override fun onBind(intent: Intent): IBinder = binder

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        try {
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.Q) {
                startForeground(
                    NOTIFICATION_ID,
                    buildNotification("T.U.S.H.E.R Sync"),
                    android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
                )
            } else {
                startForeground(NOTIFICATION_ID, buildNotification("T.U.S.H.E.R Sync"))
            }
        } catch (e: Exception) {
            android.util.Log.e("TusherForeground", "Failed to start foreground: ${e.message}")
        }

        when (intent?.action) {
            ACTION_START -> startSync()
            ACTION_PAUSE -> pauseSync()
            ACTION_STOP -> { 
                try { stopForeground(STOP_FOREGROUND_REMOVE) } catch (_: Exception) {}
                stopSelf() 
            }
        }
        return START_STICKY
    }

    private fun startSync() {
        syncJob?.cancel()
        syncJob = serviceScope.launch {
            _syncState.value = _syncState.value.copy(running = true, paused = false)
            // Polling loop — in production this would use the Rust callbacks via UniFFI
            while (isActive) {
                try {
                    val status = fetchStatusFromDesktop()
                    _syncState.value = status
                    updateNotification("Active peers: ${status.activePeers} | Files: ${status.syncedFiles}")
                } catch (e: Exception) {
                    _syncState.value = _syncState.value.copy(lastError = e.message)
                }
                delay(3_000)
            }
        }
    }

    private fun pauseSync() {
        syncJob?.cancel()
        _syncState.value = _syncState.value.copy(running = false, paused = true)
        updateNotification("Sync paused")
    }

    /**
     * Fetches status from the desktop node's REST API over Tailscale.
     * Falls back gracefully if unreachable.
     */
    private fun fetchStatusFromDesktop(): SyncState {
        return try {
            val statusUrl = java.net.URL("http://100.93.120.124:42950/api/status")
            val conn = statusUrl.openConnection() as java.net.HttpURLConnection
            conn.connectTimeout = 3000
            conn.readTimeout = 3000
            val statusJson = conn.inputStream.bufferedReader().readText()
            conn.disconnect()

            val foldersList = mutableListOf<RemoteFolderInfo>()
            try {
                val foldersUrl = java.net.URL("http://100.93.120.124:42950/api/folders")
                val fConn = foldersUrl.openConnection() as java.net.HttpURLConnection
                fConn.connectTimeout = 3000
                fConn.readTimeout = 3000
                val foldersJson = fConn.inputStream.bufferedReader().readText()
                fConn.disconnect()

                val arr = org.json.JSONArray(foldersJson)
                for (i in 0 until arr.length()) {
                    val obj = arr.getJSONObject(i)
                    foldersList.add(
                        RemoteFolderInfo(
                            folderId = obj.optString("folder_id", ""),
                            folderName = obj.optString("folder_name", "Shared Folder"),
                            fileCount = obj.optInt("file_count", 0),
                            originDevice = obj.optString("origin_device", "Desktop PC"),
                            isLocal = obj.optBoolean("is_local", false)
                        )
                    )
                }
            } catch (_: Exception) {}

            parseSyncStatus(statusJson, foldersList)
        } catch (e: Exception) {
            _syncState.value.copy(lastError = "Desktop unreachable: ${e.message?.take(60)}")
        }
    }

    private fun parseSyncStatus(json: String, foldersList: List<RemoteFolderInfo>): SyncState {
        fun field(key: String): String? {
            val pattern = Regex(""""$key"\s*:\s*([^,}\]]+)""")
            return pattern.find(json)?.groupValues?.get(1)?.trim()?.removeSurrounding("\"")
        }
        return SyncState(
            running = true,
            paused = field("sync_paused")?.toBoolean() ?: false,
            nodeId = field("node_id") ?: "unknown",
            activePeers = field("active_peers")?.toIntOrNull() ?: 0,
            sharedFolders = field("shared_folders")?.toIntOrNull() ?: foldersList.size,
            syncedFiles = 0,
            remoteFolders = foldersList,
            transferStatus = field("transfer_status") ?: "✓ Mesh Active • Synced",
            lastError = null
        )
    }

    private fun buildNotification(text: String): Notification {
        val tapIntent = Intent(this, MainActivity::class.java)
        val pi = PendingIntent.getActivity(this, 0, tapIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)

        val pauseIntent = Intent(this, TusherForegroundService::class.java).apply {
            action = ACTION_PAUSE
        }
        val pausePi = PendingIntent.getService(this, 1, pauseIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)

        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("T.U.S.H.E.R")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_menu_upload)
            .setContentIntent(pi)
            .setOngoing(true)
            .addAction(android.R.drawable.ic_media_pause, "Pause", pausePi)
            .build()
    }

    private fun updateNotification(text: String) {
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIFICATION_ID, buildNotification(text))
    }

    override fun onDestroy() {
        serviceScope.cancel()
        super.onDestroy()
    }

    companion object {
        const val CHANNEL_ID = "tusher_sync"
        const val NOTIFICATION_ID = 1001
        const val ACTION_START = "com.tusher.app.START"
        const val ACTION_PAUSE = "com.tusher.app.PAUSE"
        const val ACTION_STOP = "com.tusher.app.STOP"
    }
}

data class RemoteFolderInfo(
    val folderId: String,
    val folderName: String,
    val fileCount: Int,
    val originDevice: String,
    val isLocal: Boolean
)

data class SyncState(
    val running: Boolean = false,
    val paused: Boolean = false,
    val nodeId: String = "",
    val activePeers: Int = 0,
    val sharedFolders: Int = 0,
    val syncedFiles: Int = 0,
    val remoteFolders: List<RemoteFolderInfo> = emptyList(),
    val transferStatus: String = "✓ Mesh Active • Synced",
    val lastError: String? = null
)
