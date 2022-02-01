package dev.fanchao.cjkproxy

import android.os.Parcelable
import kotlinx.parcelize.Parcelize
import kotlinx.serialization.Serializable

typealias ProxyConfiguration = String

@Serializable
@Parcelize
data class ProxyConfigurationPersisted(
    val name: String,
    val config: ProxyConfiguration
) : Parcelable