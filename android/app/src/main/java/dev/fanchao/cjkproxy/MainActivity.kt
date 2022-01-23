package dev.fanchao.cjkproxy

import android.annotation.SuppressLint
import android.graphics.Color
import android.graphics.Typeface
import androidx.appcompat.app.AppCompatActivity
import android.os.Bundle
import android.text.SpannableStringBuilder
import android.text.style.ForegroundColorSpan
import android.text.style.StyleSpan
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import android.widget.BaseAdapter
import androidx.recyclerview.widget.DividerItemDecoration
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import dev.fanchao.CJKProxy
import dev.fanchao.cjkproxy.databinding.ActivityMainBinding
import dev.fanchao.cjkproxy.databinding.ViewConfigItemBinding

class MainActivity : AppCompatActivity() {
    private val adapter = Adapter()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val binding = ActivityMainBinding.inflate(LayoutInflater.from(this))
        setContentView(binding.root)

        binding.list.adapter = adapter
        binding.list.layoutManager = LinearLayoutManager(this)
        binding.list.addItemDecoration(DividerItemDecoration(this, DividerItemDecoration.VERTICAL))

        adapter.items = listOf(
            ProxyConfigurationPersisted("Test1", ProxyConfiguration("localhost", 80, "localhost", 80)),
            ProxyConfigurationPersisted("Test2", ProxyConfiguration("localhost", 80, "localhost", 80)),
        )
    }
}

private class VH(val binding: ViewConfigItemBinding) : RecyclerView.ViewHolder(binding.root)

private class Adapter : RecyclerView.Adapter<VH>() {
    var items = emptyList<ProxyConfigurationPersisted>()
        @SuppressLint("NotifyDataSetChanged")
        set(value) {
            if (field != value) {
                field = value
                notifyDataSetChanged()
            }
        }

    var states = mapOf<ProxyConfiguration, ProxyState>()
        @SuppressLint("NotifyDataSetChanged")
        set(value) {
            if (field != value) {
                field = value
                notifyDataSetChanged()
            }

        }

    var onItemToggled: (ProxyConfiguration, ProxyState) -> Unit = { c, s -> }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
        return VH(ViewConfigItemBinding.inflate(LayoutInflater.from(parent.context), parent, false))
    }

    override fun onBindViewHolder(holder: VH, position: Int) {
        val config = items[position]
        val state = states[config.config] ?: ProxyState.Stopped

        holder.binding.name.text = config.name
        holder.binding.status.text = SpannableStringBuilder()
            .append(state.toText(), state.styled(), 0)
            .append(" | ")
            .append(config.config.remoteHost, StyleSpan(Typeface.ITALIC), 0)
        holder.binding.toggle.setImageResource(state.image())
        holder.binding.toggle.setOnClickListener {
            onItemToggled(config.config, state)
        }
    }

    override fun getItemCount(): Int = items.size

    private fun ProxyState.image(): Int {
        return when (this) {
            is ProxyState.Running -> R.drawable.ic_stop
            else -> R.drawable.ic_play
        }
    }

    private fun ProxyState.toText(): String {
        return when (this) {
            is ProxyState.Running -> "Running"
            ProxyState.Stopped -> "Stopped"
        }
    }

    private fun ProxyState.styled(): Any? {
        return when (this) {
            is ProxyState.Running -> ForegroundColorSpan(Color.GREEN)
            ProxyState.Stopped -> null
        }

    }
}