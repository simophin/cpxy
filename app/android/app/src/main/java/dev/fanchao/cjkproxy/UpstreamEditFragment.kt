package dev.fanchao.cjkproxy

import android.os.Bundle
import android.view.LayoutInflater
import android.view.View
import android.view.ViewGroup
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.material.DropdownMenu
import androidx.compose.material.DropdownMenuItem
import androidx.compose.material.Text
import androidx.compose.material.TextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.fragment.app.Fragment

class UpstreamEditFragment : Fragment() {
    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View {
        return ComposeView(requireContext()).apply {
            setContent {
                UpstreamEdit(
                    initName = requireArguments().getString("name"),
                    initConfig = requireArguments().getParcelable("upstream")
                )
            }
        }
    }
}

data class EditingState(val text: String, val error: String?)

@Composable
fun rememberEditingState(initText: String? = null): MutableState<EditingState> {
    return remember {
        mutableStateOf(EditingState(initText.orEmpty(), null))
    }
}

@Composable
fun EditInput(s: MutableState<EditingState>, placeholder: String? = null) {
    val (state, setState) = s;
    TextField(
        value = state.text,
        isError = state.error.orEmpty().isNotBlank(),
        onValueChange = {
            setState(state.copy(text = it, error = null))
        },
        placeholder = {
            Text(placeholder.orEmpty())
        }
    )
}

@Composable
fun DropdownList(
    choices: List<String>,
    title: String,
    initSelected: List<String>,
    onSelected: (List<String>) -> Unit
) {
    val (expanded, setExpanded) = remember { mutableStateOf(false) }
    val (selected, setSelected) = remember { mutableStateOf(initSelected) }
    Box {
        if (!expanded) {
            Text(title + ": " + selected.joinToString(separator = ", "), modifier = Modifier.clickable {
                setExpanded(true)
            })
        } else {
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = {
                    setExpanded(false)
                    onSelected(selected)
                }
            ) {
                choices.map { choice ->
                    DropdownMenuItem(onClick = {
                        if (selected.contains(choice)) {
                            setSelected(selected.toMutableList().apply { remove(choice) })
                        } else {
                            setSelected(selected + listOf(choice))
                        }
                    }) {
                        Text(text = choice)
                    }
                }
            }

        }
    }
}

private val countryList = listOf("NZ", "CN")

@Composable
@Preview
private fun UpstreamEdit(initName: String? = null, initConfig: UpstreamConfig? = null) {
    val name = rememberEditingState(initName)
    val address = rememberEditingState(initConfig?.address)
    val (accept, setAccept) = remember { mutableStateOf(initConfig?.accept.orEmpty()) }
    val (reject, setReject) = remember { mutableStateOf(initConfig?.reject.orEmpty()) }
    val (matchesGfw, setMatchesGfw) = remember { mutableStateOf(initConfig?.matchGfw == true) }
    val (matchesNetworks, setMatchesNetworks) = remember { mutableStateOf(initConfig?.matchNetworks.orEmpty()) }

    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        EditInput(name, placeholder = "Name")
        EditInput(address, placeholder = "Address")
        DropdownList(choices = countryList,
            initSelected = accept, onSelected = setAccept, title = "Accepted countries: ")
    }
}