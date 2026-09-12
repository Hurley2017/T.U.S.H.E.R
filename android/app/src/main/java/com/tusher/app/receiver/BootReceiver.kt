package com.tusher.app.receiver

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.tusher.app.service.TusherForegroundService

/**
 * Starts the sync service automatically on device boot.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == Intent.ACTION_BOOT_COMPLETED) {
            val svc = Intent(context, TusherForegroundService::class.java).apply {
                action = TusherForegroundService.ACTION_START
            }
            context.startForegroundService(svc)
        }
    }
}
