package com.tusher.app.ui

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.IBinder
import android.view.LayoutInflater
import android.view.View
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.documentfile.provider.DocumentFile
import androidx.lifecycle.lifecycleScope
import com.google.android.material.button.MaterialButton
import com.tusher.app.R
import com.tusher.app.databinding.ActivityMainBinding
import com.tusher.app.service.SyncState
import com.tusher.app.service.TusherForegroundService
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

data class LocalSharedFolder(val name: String, val uriString: String, val fileCount: Int)

class MainActivity : AppCompatActivity() {

    private lateinit var binding: ActivityMainBinding
    private var syncService: TusherForegroundService? = null
    private var bound = false
    private val sharedFolders = mutableListOf<LocalSharedFolder>()

    private val openFolderLauncher = registerForActivityResult(ActivityResultContracts.OpenDocumentTree()) { uri: Uri? ->
        if (uri != null) {
            try {
                contentResolver.takePersistableUriPermission(
                    uri,
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                )
            } catch (_: Exception) {}

            val docFile = DocumentFile.fromTreeUri(this, uri)
            val name = docFile?.name ?: "Selected Folder"
            val count = docFile?.listFiles()?.size ?: 0

            sharedFolders.removeAll { it.uriString == uri.toString() }
            sharedFolders.add(LocalSharedFolder(name, uri.toString(), count))
            saveFolders()
            renderFolders()
        }
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, binder: IBinder?) {
            val lb = binder as? TusherForegroundService.LocalBinder ?: return
            syncService = lb.getService()
            bound = true
            observeSyncState()
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            syncService = null
            bound = false
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)
        setSupportActionBar(binding.toolbar)

        // Request POST_NOTIFICATIONS permission on Android 13+
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 101)
            }
        }

        loadFolders()
        renderFolders()

        binding.btnAddFolder.setOnClickListener {
            openFolderLauncher.launch(null)
        }

        // Start + bind service
        val svcIntent = Intent(this, TusherForegroundService::class.java).apply {
            action = TusherForegroundService.ACTION_START
        }
        startForegroundService(svcIntent)
        bindService(svcIntent, connection, Context.BIND_AUTO_CREATE)

        binding.btnPause.setOnClickListener {
            val pi = Intent(this, TusherForegroundService::class.java).apply {
                action = TusherForegroundService.ACTION_PAUSE
            }
            startService(pi)
        }

        binding.btnRefresh.setOnClickListener {
            val pi = Intent(this, TusherForegroundService::class.java).apply {
                action = TusherForegroundService.ACTION_START
            }
            startService(pi)
            renderFolders()
        }
    }

    private var lastRemoteFolders = listOf<com.tusher.app.service.RemoteFolderInfo>()

    private fun loadFolders() {
        val prefs = getSharedPreferences("tusher_folders", Context.MODE_PRIVATE)
        val raw = prefs.getString("folders", "") ?: ""
        sharedFolders.clear()
        if (raw.isNotEmpty()) {
            raw.split(";").filter { it.isNotEmpty() }.forEach { entry ->
                val parts = entry.split("|")
                if (parts.size >= 2) {
                    val name = parts[0]
                    val uri = parts[1]
                    val count = parts.getOrNull(2)?.toIntOrNull() ?: 0
                    sharedFolders.add(LocalSharedFolder(name, uri, count))
                }
            }
        }
    }

    private fun saveFolders() {
        val prefs = getSharedPreferences("tusher_folders", Context.MODE_PRIVATE)
        val raw = sharedFolders.joinToString(";") { "${it.name}|${it.uriString}|${it.fileCount}" }
        prefs.edit().putString("folders", raw).apply()
    }

    private fun renderFolders() {
        binding.llFolderList.removeAllViews()
        val totalFolders = sharedFolders.size + lastRemoteFolders.size
        if (totalFolders == 0) {
            binding.tvEmptyFolders.visibility = View.VISIBLE
            return
        }
        binding.tvEmptyFolders.visibility = View.GONE

        // 1. Render Local Folders (Served by this device)
        for (f in sharedFolders) {
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply { setMargins(0, 0, 0, 16) }
                setPadding(20, 20, 20, 20)
                setBackgroundResource(android.R.drawable.dialog_holo_light_frame)
            }

            val meta = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            }

            val title = TextView(this).apply {
                text = "📁  ${f.name}"
                textSize = 16f
                setTypeface(null, android.graphics.Typeface.BOLD)
            }

            val origin = TextView(this).apply {
                text = "📱 Served by Lenovo Tablet (This Device)"
                textSize = 12f
                setTextColor(resources.getColor(android.R.color.holo_blue_dark, theme))
                setTypeface(null, android.graphics.Typeface.BOLD)
                setPadding(0, 4, 0, 4)
            }

            val sub = TextView(this).apply {
                text = "${f.fileCount} files • Local Storage (SAF)"
                textSize = 12f
                setTextColor(resources.getColor(android.R.color.darker_gray, theme))
            }

            meta.addView(title)
            meta.addView(origin)
            meta.addView(sub)

            val btnRemove = MaterialButton(this, null, com.google.android.material.R.attr.borderlessButtonStyle).apply {
                text = "✕"
                textSize = 14f
                setOnClickListener {
                    sharedFolders.remove(f)
                    saveFolders()
                    renderFolders()
                }
            }

            row.addView(meta)
            row.addView(btnRemove)
            binding.llFolderList.addView(row)
        }

        // 2. Render Remote Mesh Folders
        for (rf in lastRemoteFolders) {
            val row = LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                layoutParams = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.MATCH_PARENT,
                    LinearLayout.LayoutParams.WRAP_CONTENT
                ).apply { setMargins(0, 0, 0, 16) }
                setPadding(20, 20, 20, 20)
                setBackgroundResource(android.R.drawable.dialog_holo_light_frame)
            }

            val meta = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL
                layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            }

            val title = TextView(this).apply {
                text = "📁  ${rf.folderName}"
                textSize = 16f
                setTypeface(null, android.graphics.Typeface.BOLD)
            }

            val origin = TextView(this).apply {
                text = "💻 Served by ${rf.originDevice}"
                textSize = 12f
                setTextColor(resources.getColor(android.R.color.holo_green_dark, theme))
                setTypeface(null, android.graphics.Typeface.BOLD)
                setPadding(0, 4, 0, 4)
            }

            val sub = TextView(this).apply {
                text = "${rf.fileCount} files • Auto-Synced Mesh Folder"
                textSize = 12f
                setTextColor(resources.getColor(android.R.color.darker_gray, theme))
            }

            meta.addView(title)
            meta.addView(origin)
            meta.addView(sub)

            row.addView(meta)
            binding.llFolderList.addView(row)
        }
    }

    private fun observeSyncState() {
        val svc = syncService ?: return
        lifecycleScope.launch {
            svc.syncState.collectLatest { state ->
                lastRemoteFolders = state.remoteFolders
                renderFolders()
                updateUi(state)
            }
        }
    }

    private fun updateUi(state: SyncState) {
        with(binding) {
            tvStatus.text = when {
                state.lastError != null -> "⚠ ${state.lastError}"
                state.paused -> "⏸ Sync Paused"
                state.running -> state.transferStatus
                else -> "○ Idle"
            }
            tvNodeId.text = "Mesh Host: ${state.nodeId.take(24).ifEmpty { "discovering..." }}"
            tvPeers.text = "Active Mesh Nodes: ${state.activePeers + 1}"
            tvFolders.text = "Mesh Shared Folders: ${sharedFolders.size + lastRemoteFolders.size}"
            tvFiles.text = "Files Synced: ${state.syncedFiles}"
            btnPause.text = if (state.paused) "Resume" else "Pause"
            statusIndicator.setBackgroundResource(
                when {
                    state.lastError != null -> android.R.color.holo_orange_light
                    state.running -> android.R.color.holo_green_light
                    else -> android.R.color.darker_gray
                }
            )
        }
    }

    override fun onDestroy() {
        if (bound) {
            unbindService(connection)
            bound = false
        }
        super.onDestroy()
    }
}
