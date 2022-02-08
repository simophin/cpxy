package dev.fanchao.cjkproxy.ui

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import androidx.browser.customtabs.CustomTabsIntent
import dev.fanchao.cjkproxy.Service
import dev.fanchao.cjkproxy.databinding.ActivityMainBinding
import io.reactivex.rxjava3.disposables.Disposable

class MainActivity : AppCompatActivity() {
    private lateinit var binding: ActivityMainBinding
    private var disposable: Disposable? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        binding = ActivityMainBinding.inflate(layoutInflater)
        setContentView(binding.root)

        binding.start.setOnClickListener {
            startService(Intent(this, Service::class.java))
        }

        binding.stop.setOnClickListener {
            stopService(Intent(this, Service::class.java))
        }
    }

    private fun ActivityMainBinding.updateState() {
        start.isEnabled = Service.runningPort == null
        stop.isEnabled = Service.runningPort != null
        admin.isEnabled = Service.runningPort != null
        Service.runningPort?.let { port ->
            admin.setOnClickListener {
                CustomTabsIntent.Builder().build()
                    .launchUrl(this@MainActivity, Uri.parse("http://localhost:$port"))
            }
        }
    }

    override fun onStart() {
        super.onStart()

        disposable = Service.notifications
            .startWithItem(Unit)
            .subscribe {
                binding.updateState()
            }
    }

    override fun onStop() {
        super.onStop()

        disposable?.dispose()
    }
}