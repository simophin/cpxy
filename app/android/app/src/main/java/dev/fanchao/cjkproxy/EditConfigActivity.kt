package dev.fanchao.cjkproxy

import android.content.Intent
import android.os.Bundle
import android.view.LayoutInflater
import android.view.Menu
import android.view.MenuItem
import androidx.appcompat.app.AppCompatActivity
import com.google.android.material.textfield.TextInputEditText
import dev.fanchao.CJKProxy
import dev.fanchao.cjkproxy.databinding.ActivityAddConfigBinding

class EditConfigActivity : AppCompatActivity() {
    private lateinit var binding: ActivityAddConfigBinding
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        binding = ActivityAddConfigBinding.inflate(LayoutInflater.from(this))
        setContentView(binding.root)

        val config: ProxyConfigurationPersisted? = intent.getParcelableExtra(EXTRA_CONFIG)
        binding.configEdit.setText(config?.config.orEmpty())
        binding.configName.setText(config?.name.orEmpty())
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
        val config = binding.configEdit.text.toString()
        val msg = try {
            CJKProxy.verifyConfig(config)
            null
        } catch (ec: Throwable) {
            ec.message.orEmpty()
        }

        val name = binding.configName.text.toString()

        if (msg == null) {
            setResult(
                RESULT_OK, Intent()
                    .putExtra(EXTRA_CONFIG, intent.getParcelableExtra<ProxyConfigurationPersisted>(EXTRA_CONFIG))
                    .putExtra(EXTRA_UPDATED_CONFIG, ProxyConfigurationPersisted(name, config))
            )
        }
    }


    companion object {
        const val EXTRA_CONFIG = "config"
        const val EXTRA_UPDATED_CONFIG = "updated"
    }
}