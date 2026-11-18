package dev.cloister.app.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import dev.cloister.app.R

/** The design tokens (spec #1, "UI and design system"). Day and Night are driven by time. */
@Immutable
data class CloisterColors(
    val ground: Color,
    val surface: Color,
    val sunk: Color,
    val line: Color,
    val ink: Color,
    val ink2: Color,
    val ink3: Color,
    val accent: Color,
    val brass: Color,
    val brassTint: Color,
    val night: Boolean,
)

/** One hue, five depths; also the Depth Rating scale. No red anywhere. */
object Depth {
    val d1 = Color(0xFFC9D9D6)
    val d2 = Color(0xFF94B3AF)
    val d3 = Color(0xFF4A7A76)
    val d4 = Color(0xFF2F5F5E)
    val d5 = Color(0xFF123837)
    val all = listOf(d1, d2, d3, d4, d5)
}

val Day = CloisterColors(
    ground = Color(0xFFEEF0EE), surface = Color(0xFFF7F8F6), sunk = Color(0xFFE3E6E3), line = Color(0xFFCDD2CE),
    ink = Color(0xFF17211F), ink2 = Color(0xFF47534F), ink3 = Color(0xFF56625E), accent = Depth.d4,
    brass = Color(0xFF7A5A1E), brassTint = Color(0xFFEFE7D3), night = false,
)

val Night = CloisterColors(
    ground = Color(0xFF0E1716), surface = Color(0xFF15201F), sunk = Color(0xFF1C2A29), line = Color(0xFF2B3B39),
    ink = Color(0xFFE4EAE7), ink2 = Color(0xFFA9B7B3), ink3 = Color(0xFFA9B7B3), accent = Color(0xFF8FB8B2),
    brass = Color(0xFFC9A968), brassTint = Color(0xFF2B2A1F), night = true,
)

val Garamond = FontFamily(
    Font(R.font.ebgaramond_regular, FontWeight.Normal),
    Font(R.font.ebgaramond_medium, FontWeight.Medium),
)
val SourceSans = FontFamily(
    Font(R.font.source_sans3_regular, FontWeight.Normal),
    Font(R.font.source_sans3_medium, FontWeight.Medium),
    Font(R.font.source_sans3_semibold, FontWeight.SemiBold),
)
val SourceCode = FontFamily(
    Font(R.font.source_code_pro_regular, FontWeight.Normal),
    Font(R.font.source_code_pro_medium, FontWeight.Medium),
)

/** Type ramp: Garamond for Display/Title/Intent, Source Sans 3 for UI, Source Code Pro for figures. */
object Type {
    val display = TextStyle(fontFamily = Garamond, fontWeight = FontWeight.Medium, fontSize = 44.sp, lineHeight = 48.sp, letterSpacing = (-0.01).em)
    val title = TextStyle(fontFamily = Garamond, fontWeight = FontWeight.Medium, fontSize = 34.sp, lineHeight = 38.sp)
    val intent = TextStyle(fontFamily = Garamond, fontWeight = FontWeight.Medium, fontSize = 34.sp, lineHeight = 40.sp)
    val heading = TextStyle(fontFamily = SourceSans, fontWeight = FontWeight.SemiBold, fontSize = 20.sp, lineHeight = 28.sp)
    val body = TextStyle(fontFamily = SourceSans, fontWeight = FontWeight.Normal, fontSize = 16.sp, lineHeight = 24.sp)
    val detail = TextStyle(fontFamily = SourceSans, fontWeight = FontWeight.Normal, fontSize = 15.sp, lineHeight = 22.sp)
    val label = TextStyle(fontFamily = SourceSans, fontWeight = FontWeight.Medium, fontSize = 13.sp, lineHeight = 18.sp)
    val eyebrow = TextStyle(fontFamily = SourceSans, fontWeight = FontWeight.SemiBold, fontSize = 12.sp, lineHeight = 16.sp, letterSpacing = 0.12.em)
    val timer = TextStyle(fontFamily = SourceCode, fontWeight = FontWeight.Medium, fontSize = 72.sp, lineHeight = 76.sp, letterSpacing = (-0.03).em)
    val figure = TextStyle(fontFamily = SourceCode, fontWeight = FontWeight.Medium, fontSize = 15.sp, lineHeight = 22.sp)
}

val LocalColors = staticCompositionLocalOf { Day }

/** Material 3 roles mapped onto the tokens, so stock components pick up the design. */
private fun scheme(c: CloisterColors): ColorScheme {
    val base = if (c.night) darkColorScheme() else lightColorScheme()
    return base.copy(
        primary = if (c.night) c.accent else Depth.d4,
        onPrimary = if (c.night) c.ground else c.surface,
        primaryContainer = Depth.d1,
        onPrimaryContainer = Depth.d5,
        secondary = c.brass,
        onSecondary = c.surface,
        secondaryContainer = c.brassTint,
        onSecondaryContainer = c.ink,
        tertiary = Depth.d3,
        background = c.ground,
        onBackground = c.ink,
        surface = c.surface,
        onSurface = c.ink,
        surfaceVariant = c.sunk,
        onSurfaceVariant = c.ink2,
        surfaceContainer = c.surface,
        surfaceContainerLow = c.surface,
        surfaceContainerHigh = c.sunk,
        surfaceContainerHighest = c.sunk,
        outline = c.line,
        outlineVariant = c.line,
        // No red anywhere: "error" roles reuse the brass record colour.
        error = c.brass,
        onError = c.surface,
        errorContainer = c.brassTint,
        onErrorContainer = c.ink,
    )
}

private val shapes = Shapes(
    extraSmall = RoundedCornerShape(2.dp),
    small = RoundedCornerShape(2.dp),
    medium = RoundedCornerShape(4.dp),
    large = RoundedCornerShape(4.dp),
    extraLarge = RoundedCornerShape(4.dp),
)

@Composable
fun CloisterTheme(night: Boolean = false, content: @Composable () -> Unit) {
    val colors = if (night) Night else Day
    CompositionLocalProvider(LocalColors provides colors) {
        MaterialTheme(
            colorScheme = scheme(colors),
            shapes = shapes,
            typography = Typography(
                displayLarge = Type.display, headlineLarge = Type.title, titleLarge = Type.heading,
                bodyLarge = Type.body, bodyMedium = Type.detail, labelLarge = Type.label, labelSmall = Type.eyebrow,
            ),
            content = content,
        )
    }
}
