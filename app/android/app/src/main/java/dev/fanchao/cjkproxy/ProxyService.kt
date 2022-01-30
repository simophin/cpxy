package dev.fanchao.cjkproxy

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import io.reactivex.rxjava3.disposables.Disposable

class ProxyService : Service() {
    private lateinit var disposable: Disposable

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            (getSystemService(NOTIFICATION_SERVICE) as NotificationManager).createNotificationChannel(
                NotificationChannel("default", "Default", NotificationManager.IMPORTANCE_DEFAULT)
            )
        }

        disposable = App.instance.proxyInstances.observeInstances()
            .subscribe {
                if (it.isNotEmpty()) {
                    startForeground(
                        1, NotificationCompat.Builder(this, "default")
                            .setContentTitle("${it.size} proxy running")
                            .setPriority(NotificationCompat.PRIORITY_LOW)
                            .setSmallIcon(R.drawable.ic_launcher_foreground)
                            .setContentIntent(
                                PendingIntent.getActivity(
                                    this,
                                    0,
                                    Intent(this, MainActivity::class.java),
                                    PendingIntent.FLAG_IMMUTABLE
                                )
                            )
                            .addAction(
                                R.drawable.ic_stop, "Stop all", PendingIntent.getService(
                                    this,
                                    1,
                                    Intent(this, this.javaClass)
                                        .setAction("${packageName}.stop"),
                                    PendingIntent.FLAG_IMMUTABLE
                                )
                            )
                            .build()
                    )
                } else {
                    stopForeground(true);
                }
            }
    }

    override fun onDestroy() {
        super.onDestroy()

        disposable.dispose()
    }

    override fun onStartCommand(intent: Intent, flags: Int, startId: Int): Int {
        if (intent.action == "${packageName}.stop") {
            App.instance.proxyInstances.updateInstances(emptySet())
        }
        return super.onStartCommand(intent, flags, startId)
    }
}