package dev.cloister.app.ui

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import dev.cloister.app.R
import kotlinx.coroutines.launch

private val Cut = RoundedCornerShape(2.dp)

/** Logo and an eyebrow in brass capitals (the only capitals). */
@Composable
fun Masthead(eyebrow: String) {
    val c = LocalColors.current
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        Image(painterResource(R.mipmap.ic_launcher), contentDescription = "Cloister", modifier = Modifier.size(24.dp))
        Eyebrow(eyebrow, c.brass)
    }
}

@Composable
fun Eyebrow(text: String, color: Color = LocalColors.current.brass) {
    Text(text.uppercase(), style = Type.eyebrow, color = color)
}

/** Depth 4 fill: the commit action. */
@Composable
fun CommitButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .clip(Cut)
            .background(if (enabled) Depth.d4 else c.sunk)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = Type.detail.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold), color = if (enabled) Color.White else c.ink3)
    }
}

/** Outlined, for everything that isn't a commitment. */
@Composable
fun QuietButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier
            .heightIn(min = 48.dp)
            .clip(Cut)
            .border(BorderStroke(1.dp, if (enabled) c.ink else c.line), Cut)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = Type.detail.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold), color = if (enabled) c.ink else c.ink3)
    }
}

@Composable
fun TextAction(text: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
    Text(
        text,
        style = Type.detail.copy(textDecoration = TextDecoration.Underline),
        color = LocalColors.current.ink2,
        modifier = modifier
            .heightIn(min = 48.dp)
            .clickable(role = Role.Button, onClick = onClick)
            .padding(vertical = 13.dp),
    )
}

/**
 * Hold to commit: a 2 s press while the Depth 5 fill sinks from the top. Letting go early
 * drains it. Screen readers get a plain action ("Hold to begin" activates at once).
 */
@Composable
fun HoldToCommit(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onCommit: () -> Unit) {
    val c = LocalColors.current
    val fill = remember { Animatable(0f) }
    val scope = rememberCoroutineScope()
    Box(
        modifier
            .fillMaxWidth()
            .height(56.dp)
            .clip(Cut)
            .background(if (enabled) Depth.d4 else c.sunk)
            .semantics {
                role = Role.Button
                contentDescription = "$text. Press and hold for two seconds."
                onClick(label = text) { if (enabled) onCommit(); enabled }
            }
            .pointerInput(enabled) {
                if (!enabled) return@pointerInput
                awaitEachGesture {
                    awaitFirstDown()
                    val sinking = scope.launch {
                        fill.animateTo(1f, tween(durationMillis = ((1f - fill.value) * 2000).toInt(), easing = LinearEasing))
                        onCommit()
                        fill.snapTo(0f)
                    }
                    waitForUpOrCancellation()
                    if (fill.value < 1f) {
                        sinking.cancel()
                        scope.launch { fill.animateTo(0f, tween(240)) }
                    }
                }
            },
    ) {
        Box(Modifier.fillMaxWidth().fillMaxHeight(fill.value).background(Depth.d5))
        Text(
            text,
            style = Type.detail.copy(fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold),
            color = if (enabled) Color.White else c.ink3,
            modifier = Modifier.align(Alignment.Center),
        )
    }
}

/** The Intent: serif text on a single rule. */
@Composable
fun IntentField(value: String, onValueChange: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Column(modifier) {
        Box {
            if (value.isEmpty()) Text(placeholder, style = Type.intent, color = c.ink3)
            BasicTextField(
                value = value,
                onValueChange = { onValueChange(it.replace("\n", " ")) },
                textStyle = Type.intent.copy(color = c.ink),
                cursorBrush = SolidColor(Depth.d3),
                keyboardOptions = KeyboardOptions(capitalization = androidx.compose.ui.text.input.KeyboardCapitalization.Sentences),
                modifier = Modifier.fillMaxWidth().semantics { contentDescription = "Intent" },
            )
        }
        Box(Modifier.padding(top = 8.dp).fillMaxWidth().height(1.dp).background(c.ink2))
    }
}

/** A plain one-line field on a rule (email, code). */
@Composable
fun LineField(value: String, onValueChange: (String) -> Unit, label: String, style: TextStyle = Type.body, keyboard: KeyboardOptions = KeyboardOptions.Default) {
    val c = LocalColors.current
    Column {
        Text(label, style = Type.label, color = c.ink2)
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            singleLine = true,
            textStyle = style.copy(color = c.ink),
            cursorBrush = SolidColor(Depth.d3),
            keyboardOptions = keyboard,
            modifier = Modifier.fillMaxWidth().padding(vertical = 10.dp).semantics { contentDescription = label },
        )
        Box(Modifier.fillMaxWidth().height(1.dp).background(c.ink2))
    }
}

/** A tag: cut corners, never a pill. */
@Composable
fun Tag(text: String, background: Color = Depth.d1, color: Color = Depth.d5) {
    Text(
        text,
        style = Type.label,
        color = color,
        modifier = Modifier.clip(Cut).background(background).padding(horizontal = 6.dp, vertical = 2.dp),
    )
}

/** Sunk for "what is so"; brass tint for "what was recorded". Never an error style. */
@Composable
fun Notice(text: String, recorded: Boolean = false, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Text(
        text,
        style = Type.detail,
        color = c.ink,
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(4.dp))
            .background(if (recorded) c.brassTint else c.sunk)
            .padding(12.dp),
    )
}

/** Status: a word beside a square. No dots, no badges; the word carries the meaning. */
@Composable
fun Status(word: String, on: Boolean) {
    val c = LocalColors.current
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        Box(Modifier.size(10.dp).background(if (on) Depth.d4 else c.line))
        Text(word, style = Type.label, color = c.ink2)
    }
}

/** A 4 px progress line: Depth 4 over Sunk. */
@Composable
fun Progress(fraction: Float, description: String) {
    val c = LocalColors.current
    Box(Modifier.fillMaxWidth().height(4.dp).background(c.sunk).semantics { contentDescription = description }) {
        Box(Modifier.fillMaxWidth(fraction.coerceIn(0f, 1f)).height(4.dp).background(Depth.d4))
    }
}

/** Depth Rating: five squares along the depth ramp. */
@Composable
fun DepthRatingSquares(selected: Int?, onSelect: (Int) -> Unit) {
    val c = LocalColors.current
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (n in 1..5) {
            val chosen = selected == n
            Box(
                Modifier
                    .size(56.dp)
                    .background(Depth.all[n - 1])
                    .border(if (chosen) 3.dp else 0.dp, if (chosen) c.ink else Color.Transparent)
                    .clickable(role = Role.RadioButton) { onSelect(n) }
                    .semantics {
                        contentDescription = "Depth $n of 5"
                        stateDescription = if (chosen) "Selected" else "Not selected"
                    },
                contentAlignment = Alignment.Center,
            ) {
                Text("$n", style = Type.figure, color = if (n >= 3) Color.White else Depth.d5)
            }
        }
    }
}

/** A square choice (lengths): Depth 1 tint when chosen, a line otherwise. */
@Composable
fun Choice(text: String, chosen: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier
            .heightIn(min = 48.dp)
            .clip(Cut)
            .background(if (chosen) Depth.d1 else Color.Transparent)
            .border(1.dp, if (chosen) Depth.d3 else c.line, Cut)
            .clickable(role = Role.RadioButton, onClick = onClick)
            .semantics { stateDescription = if (chosen) "Selected" else "Not selected" }
            .padding(horizontal = 14.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, style = Type.figure, color = if (chosen) Depth.d5 else c.ink)
    }
}

/** A square check for on/off choices (no switches: they are pills). */
@Composable
fun SquareCheck(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .heightIn(min = 48.dp)
            .clickable(role = Role.Checkbox) { onChange(!checked) }
            .semantics { stateDescription = if (checked) "On" else "Off" },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Box(
            Modifier.size(22.dp).clip(Cut).border(1.dp, c.ink2, Cut).background(if (checked) Depth.d4 else Color.Transparent),
            contentAlignment = Alignment.Center,
        ) {
            if (checked) Box(Modifier.size(8.dp).background(Color.White))
        }
        Text(label, style = Type.body, color = c.ink)
    }
}
