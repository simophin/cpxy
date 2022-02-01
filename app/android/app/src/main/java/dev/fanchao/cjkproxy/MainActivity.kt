@file:Suppress("DEPRECATION")

package dev.fanchao.cjkproxy

import android.annotation.SuppressLint
import android.content.Intent
import android.graphics.Color
import android.os.Bundle
import android.preference.PreferenceManager
import android.text.SpannableStringBuilder
import android.text.style.ForegroundColorSpan
import android.view.*
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.recyclerview.widget.DividerItemDecoration
import androidx.recyclerview.widget.LinearLayoutManager
import androidx.recyclerview.widget.RecyclerView
import com.afollestad.materialdialogs.MaterialDialog
import dev.fanchao.cjkproxy.databinding.ActivityMainBinding
import dev.fanchao.cjkproxy.databinding.ViewConfigItemBinding
import io.reactivex.rxjava3.android.schedulers.AndroidSchedulers
import io.reactivex.rxjava3.core.Observable
import io.reactivex.rxjava3.core.Single
import io.reactivex.rxjava3.disposables.CompositeDisposable
import io.reactivex.rxjava3.schedulers.Schedulers
import io.reactivex.rxjava3.subjects.BehaviorSubject
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import androidx.appcompat.view.ActionMode

class MainActivity : AppCompatActivity() {
    private val adapter = Adapter(this::onItemToggled, this::onItemClicked, this::onItemLongClicked)
    private var disposables: CompositeDisposable? = null
    private lateinit var binding: ActivityMainBinding
    private val reloadNotification = BehaviorSubject.createDefault(Unit)
    private var actionMode: ActionMode? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        binding = ActivityMainBinding.inflate(LayoutInflater.from(this))
        setContentView(binding.root)

        binding.list.adapter = adapter
        binding.list.layoutManager = LinearLayoutManager(this)
        binding.list.addItemDecoration(DividerItemDecoration(this, DividerItemDecoration.VERTICAL))

        startService(Intent(this, ProxyService::class.java))
    }

    override fun onStart() {
        super.onStart()

        val disposables = CompositeDisposable()

        if (adapter.items.isEmpty()) {
            binding.progress.show()
        }
        binding.empty.visibility = View.GONE

        Observable.combineLatest(
            reloadNotification
                .map {
                    PreferenceManager.getDefaultSharedPreferences(this)
                        .getString("configs", null)
                        ?.let { Json.decodeFromString<Array<ProxyConfigurationPersisted>>(it) }
                        ?.toList()
                        .orEmpty()
                }
                .subscribeOn(Schedulers.computation()),
            App.instance.proxyInstances
                .observeInstances()
        ) { first, second -> first to second }
            .observeOn(AndroidSchedulers.mainThread())
            .subscribe({ (list, states) ->
                adapter.items = list
                adapter.states = states
                binding.empty.visibility = if (list.isEmpty()) View.VISIBLE else View.GONE
                binding.progress.hide()
            }, { err ->
                Toast.makeText(
                    this,
                    "Error loading configs: ${err.message.orEmpty()}",
                    Toast.LENGTH_LONG
                ).show()
                binding.progress.hide()
            })
            .also(disposables::add)

        this.disposables = disposables
    }

    override fun onStop() {
        super.onStop()

        disposables?.dispose()
        disposables = null
    }

    override fun onCreateOptionsMenu(menu: Menu): Boolean {
        menuInflater.inflate(R.menu.main, menu)
        return super.onCreateOptionsMenu(menu)
    }

    override fun onOptionsItemSelected(item: MenuItem): Boolean {
        if (item.itemId == R.id.menu_add) {
            startActivityForResult(Intent(this, EditConfigActivity::class.java), 0)
            return true
        }
        return super.onOptionsItemSelected(item)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == 0 && resultCode == RESULT_OK) {
            val oldConfig: ProxyConfigurationPersisted? =
                data?.getParcelableExtra(EditConfigActivity.EXTRA_CONFIG)
            val newConfig: ProxyConfigurationPersisted =
                checkNotNull(data?.getParcelableExtra(EditConfigActivity.EXTRA_UPDATED_CONFIG))

            val indexOfOldConfig = adapter.items.indexOf(oldConfig)
            if (indexOfOldConfig < 0) {
                adapter.items = adapter.items + listOf(newConfig)
            } else {
                adapter.items =
                    adapter.items.toMutableList().apply { this[indexOfOldConfig] = newConfig }
            }

            doSaveAdapterItems()
            return
        }
        super.onActivityResult(requestCode, resultCode, data)
    }

    private fun onItemToggled(c: ProxyConfigurationPersisted, state: ProxyState) {
        if (state == ProxyState.Stopped) {
            App.instance.proxyInstances.updateInstances(
                adapter.states
                    .asSequence()
                    .mapNotNull { if (it.value is ProxyState.Running) it.key else null }
                    .toMutableSet().apply {
                        add(c.config)
                    }
            )
        } else if (state is ProxyState.Running) {
            App.instance.proxyInstances.updateInstances(
                adapter.states
                    .asSequence()
                    .mapNotNull { if (it.value is ProxyState.Running) it.key else null }
                    .toMutableSet().apply {
                        remove(c.config)
                    }
            )
        }
    }

    private fun onItemClicked(c: ProxyConfigurationPersisted) {
        when {
            actionMode == null -> {
                startActivityForResult(
                    Intent(this, EditConfigActivity::class.java)
                        .putExtra(EditConfigActivity.EXTRA_CONFIG, c),
                    0
                )
            }
            adapter.selectedConfigs.contains(c) -> {
                adapter.selectedConfigs = adapter.selectedConfigs.toMutableSet().apply { remove(c) }
                actionMode?.invalidate()
            }
            else -> {
                adapter.selectedConfigs = adapter.selectedConfigs.toMutableSet().apply { add(c) }
                actionMode?.invalidate()
            }
        }
    }

    private fun onItemLongClicked(c: ProxyConfigurationPersisted): Boolean {
        if (actionMode == null) {
            adapter.selectedConfigs = setOf(c)
            actionMode = startSupportActionMode(object : ActionMode.Callback {
                override fun onCreateActionMode(mode: ActionMode?, menu: Menu): Boolean {
                    menuInflater.inflate(R.menu.main_action, menu)
                    return true
                }

                override fun onPrepareActionMode(mode: ActionMode?, menu: Menu): Boolean {
                    menu.findItem(R.id.menu_delete)?.isEnabled = adapter.selectedConfigs.isNotEmpty()
                    return true
                }

                override fun onActionItemClicked(mode: ActionMode?, item: MenuItem?): Boolean {
                    if (item?.itemId == R.id.menu_delete) {
                        confirmConfigDeletion()
                    }
                    return true
                }

                override fun onDestroyActionMode(mode: ActionMode?) {
                    adapter.selectedConfigs = emptySet()
                    actionMode = null
                }
            })
            return true
        }

        return false
    }

    private fun confirmConfigDeletion() {
        MaterialDialog(this).show {
            title(text = "Delete configs")
            message(text = "About to delete ${adapter.selectedConfigs.size} configurations")
            positiveButton {
                doConfigDeletion()
                actionMode?.finish()
                it.dismiss()
            }
            negativeButton {
                it.dismiss()
            }
        }
    }

    private fun doConfigDeletion() {
        adapter.items = adapter.items.toMutableList().apply {
            removeAll(adapter.selectedConfigs)
        }
        doSaveAdapterItems()
    }

    private fun doSaveAdapterItems() {
        Single.fromCallable {
            val items = adapter.items

            PreferenceManager.getDefaultSharedPreferences(App.instance)
                .edit()
                .putString("configs", Json.encodeToString(items))
                .apply()

            items
        }.subscribeOn(Schedulers.computation())
            .observeOn(AndroidSchedulers.mainThread())
            .subscribe({
                Toast.makeText(
                    App.instance,
                    "Configuration saved",
                    Toast.LENGTH_LONG
                ).show()

                App.instance.proxyInstances
                    .updateInstances(adapter.items.mapNotNullTo(mutableSetOf()) {
                        if (adapter.states[it.config] is ProxyState.Running) it.config else null
                    })

                reloadNotification.onNext(Unit)
            }, { err ->
                Toast.makeText(
                    App.instance,
                    "Error saving configuration: ${err.message ?: "Unknown error"}",
                    Toast.LENGTH_LONG
                ).show()
            })

    }
}

private class VH(val binding: ViewConfigItemBinding) : RecyclerView.ViewHolder(binding.root)

private class Adapter(
    private val onItemToggled: (ProxyConfigurationPersisted, ProxyState) -> Unit,
    private val onItemClicked: (ProxyConfigurationPersisted) -> Unit,
    private val onItemLongClicked: (ProxyConfigurationPersisted) -> Boolean,
) : RecyclerView.Adapter<VH>() {
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

    var selectedConfigs = emptySet<ProxyConfigurationPersisted>()
        set(value) {
            @SuppressLint("NotifyDataSetChanged")
            if (field != value) {
                field = value
                notifyDataSetChanged()
            }
        }

    override fun onCreateViewHolder(parent: ViewGroup, viewType: Int): VH {
        return VH(ViewConfigItemBinding.inflate(LayoutInflater.from(parent.context), parent, false))
    }

    override fun onBindViewHolder(holder: VH, position: Int) {
        val config = items[position]
        val state = states[config.config] ?: ProxyState.Stopped
        val isSelected = selectedConfigs.contains(config)

        holder.binding.name.text = SpannableStringBuilder()
            .append(config.name)
            .append(" (")
            .append(state.toText(), state.styled(), 0)
            .append(")")

        holder.binding.toggle.setImageResource(state.image())
        holder.binding.toggle.setOnClickListener {
            if (!isSelected) onItemToggled(config, state)
        }
        holder.binding.root.setOnClickListener { onItemClicked(config) }
        holder.binding.root.setOnLongClickListener {
            onItemLongClicked(config)
        }
        holder.binding.root.setBackgroundResource(if (isSelected) com.afollestad.materialdialogs.R.drawable.md_item_selected else com.afollestad.materialdialogs.R.drawable.md_item_selector)
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