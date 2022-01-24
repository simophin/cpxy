package dev.fanchao.cjkproxy

import android.os.Parcelable
import kotlinx.parcelize.Parcelize
import kotlinx.serialization.Serializable

@Parcelize
@Serializable
data class ProxyConfiguration(
    val remoteHost: String,
    val remotePort: Int,
    val socksHost: String,
    val socksPort: Int,
): Parcelable


@Serializable
@Parcelize
data class ProxyConfigurationPersisted(
    val name: String,
    val config: ProxyConfiguration
) : Parcelable