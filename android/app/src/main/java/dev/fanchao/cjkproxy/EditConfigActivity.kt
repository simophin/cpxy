package dev.fanchao.cjkproxy

import android.content.Intent
import android.os.Bundle
import android.view.LayoutInflater
import android.view.Menu
import android.view.MenuItem
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.textfield.TextInputEditText
import dev.fanchao.cjkproxy.databinding.ActivityAddConfigBinding

class EditConfigActivity : AppCompatActivity() {
    private lateinit var binding: ActivityAddConfigBinding
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        binding = ActivityAddConfigBinding.inflate(LayoutInflater.from(this))
        setContentView(binding.root)

        val config: ProxyConfigurationPersisted? = intent.getParcelableExtra(EXTRA_CONFIG)
        if (config != null) {
            binding.name.setText(config.name)
            binding.remoteHost.setText(config.config.remoteHost)
            binding.remotePort.setText(config.config.remotePort.toString())
            binding.socks5Host.setText(config.config.socksHost)
            binding.socks5Port.setText(config.config.socksPort.toString())
        } else {
            binding.socks5Port.setText(App.instance.proxyInstances.pickNewLocalPort().toString())
        }
    }

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        menu.add("Save")
            .apply { setShowAsAction(MenuItem.SHOW_AS_ACTION_ALWAYS) }
            .setOnMenuItemClickListener { doSave(); true }
        return super.onCreateOptionsMenu(menu)
    }

    private fun TextInputEditText.checkNotEmpty(): String? {
        if (length() == 0) {
            error = "Mandatory field"
            return null
        }
        return text.toString()
    }

    private fun doSave() {
        setResult(
            RESULT_OK, Intent()
            .putExtra(EXTRA_CONFIG, intent.getParcelableExtra<ProxyConfigurationPersisted>(EXTRA_CONFIG))
            .putExtra(EXTRA_UPDATED_CONFIG, ProxyConfigurationPersisted(
            binding.name.checkNotEmpty() ?: return,
            ProxyConfiguration(
                binding.remoteHost.checkNotEmpty() ?: return,
                binding.remotePort.checkNotEmpty()?.toInt() ?: return,
                binding.socks5Host.checkNotEmpty() ?: return,
                binding.socks5Port.checkNotEmpty()?.toInt() ?: return,
            )
        )))
        finish()
    }


    companion object {
        const val EXTRA_CONFIG = "config"
        const val EXTRA_UPDATED_CONFIG = "updated"
    }
}