package dev.fanchao.cjkproxy

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Surface
import androidx.compose.material.Text
import androidx.compose.material.TextField
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.lifecycle.ViewModel
import dev.fanchao.cjkproxy.ui.theme.CJKProxyTheme

class EditViewModel(): ViewModel() {
    var name: String
}


class EditActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            CJKProxyTheme {
                // A surface container using the 'background' color from the theme
                Surface(
                    modifier = Modifier.fillMaxSize(),
                    color = MaterialTheme.colors.background
                ) {
                    Greeting("Android")
                }
            }
        }
    }
}

@Composable
fun Greeting(name: String, onNameChanged: (String) -> Unit) {
    Column {
        Text("Name")
        TextField(value = name, onValueChange = onNameChanged)
    }
}

@Preview(showBackground = true)
@Composable
fun DefaultPreview() {
    CJKProxyTheme {
        Greeting("Android")
    }
}