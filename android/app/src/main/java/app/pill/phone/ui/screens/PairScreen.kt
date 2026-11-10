package app.pill.phone.ui.screens

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Computer
import androidx.compose.material.icons.rounded.ErrorOutline
import androidx.compose.material.icons.rounded.Keyboard
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.Refresh
import androidx.compose.material.icons.rounded.Settings
import androidx.compose.material.icons.rounded.Wifi
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.pill.phone.R
import app.pill.phone.ui.PairError
import app.pill.phone.ui.PairUi
import app.pill.phone.ui.components.Banner
import app.pill.phone.ui.components.BlobState
import app.pill.phone.ui.components.ButtonKind
import app.pill.phone.ui.components.Chip
import app.pill.phone.ui.components.GlassCard
import app.pill.phone.ui.components.NavBarSpacer
import app.pill.phone.ui.components.PillBackground
import app.pill.phone.ui.components.PillButton
import app.pill.phone.ui.components.StateBlob
import app.pill.phone.ui.components.StepRow
import app.pill.phone.ui.components.TopBar
import app.pill.phone.ui.components.staggerIn
import app.pill.phone.ui.theme.Motion
import app.pill.phone.ui.theme.Pill

class PairActions(
    val onBack: () -> Unit = {},
    val onScan: () -> Unit = {},
    val onManual: (Boolean) -> Unit = {},
    val onAddress: (String) -> Unit = {},
    val onCode: (String) -> Unit = {},
    val onSubmit: () -> Unit = {},
    val onConfirm: () -> Unit = {},
    val onRetry: () -> Unit = {},
)

@Composable
fun PairScreen(ui: PairUi, actions: PairActions) {
    PillBackground {
        Column(Modifier.fillMaxSize()) {
            TopBar(stringResource(R.string.pair_title), actions.onBack)
            AnimatedContent(
                ui,
                modifier = Modifier.weight(1f),
                contentKey = { it::class to ((it as? PairUi.Entry)?.manual) },
                transitionSpec = {
                    (fadeIn(Motion.calm()) + scaleIn(Motion.soft(), initialScale = 0.94f)) togetherWith (fadeOut(Motion.calm()) + scaleOut(Motion.calm(), targetScale = 0.96f))
                },
                label = "pair",
            ) { state ->
                Column(
                    Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    when (state) {
                        is PairUi.Entry -> if (state.manual) ManualEntry(state, actions) else ChooseMethod(state, actions)
                        is PairUi.Confirm -> Confirm(state, actions)
                        is PairUi.Working -> Status(BlobState.Connecting, stringResource(R.string.pair_working), state.address)
                        is PairUi.Success -> Status(BlobState.Connected, stringResource(R.string.pair_success, state.pcName), stringResource(R.string.pair_success_body))
                        is PairUi.Failed -> Failure(state, actions)
                    }
                    NavBarSpacer()
                }
            }
        }
    }
}

@Composable
private fun ChooseMethod(state: PairUi.Entry, a: PairActions) {
    val c = Pill.colors
    StateBlob(BlobState.Idle, Modifier.staggerIn(0), size = 168.dp)
    Text(stringResource(R.string.pair_headline), Modifier.staggerIn(1), style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
    Spacer(Modifier.height(18.dp))
    state.error?.let { ErrorBanner(it); Spacer(Modifier.height(14.dp)) }
    GlassCard(Modifier.fillMaxWidth().staggerIn(2)) {
        StepRow(Icons.Rounded.Settings, stringResource(R.string.pair_step1_title), stringResource(R.string.pair_step1_body))
        Spacer(Modifier.height(16.dp))
        StepRow(Icons.Rounded.QrCodeScanner, stringResource(R.string.pair_step2_title), stringResource(R.string.pair_step2_body))
        Spacer(Modifier.height(16.dp))
        StepRow(Icons.Rounded.Wifi, stringResource(R.string.pair_step3_title), stringResource(R.string.pair_step3_body))
    }
    Spacer(Modifier.height(18.dp))
    Column(Modifier.fillMaxWidth().staggerIn(3), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        PillButton(stringResource(R.string.btn_scan_qr), a.onScan, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
        PillButton(stringResource(R.string.btn_enter_code), { a.onManual(true) }, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Keyboard)
    }
}

@Composable
private fun ManualEntry(state: PairUi.Entry, a: PairActions) {
    val c = Pill.colors
    Spacer(Modifier.height(8.dp))
    Text(stringResource(R.string.pair_manual_title), Modifier.staggerIn(0), style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
    Spacer(Modifier.height(8.dp))
    Text(stringResource(R.string.pair_manual_body), Modifier.staggerIn(1), style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
    Spacer(Modifier.height(22.dp))
    state.error?.let { ErrorBanner(it); Spacer(Modifier.height(14.dp)) }
    GlassCard(Modifier.fillMaxWidth().staggerIn(2)) {
        PillTextField(state.address, a.onAddress, stringResource(R.string.field_address), stringResource(R.string.field_address_hint), KeyboardType.Uri, ImeAction.Next)
        Spacer(Modifier.height(18.dp))
        PillTextField(state.code, a.onCode, stringResource(R.string.field_code), stringResource(R.string.field_code_hint), KeyboardType.Ascii, ImeAction.Done, mono = true, capitalize = true, onDone = a.onSubmit)
    }
    Spacer(Modifier.height(18.dp))
    Column(Modifier.fillMaxWidth().staggerIn(3), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        PillButton(stringResource(R.string.btn_pair), a.onSubmit, Modifier.fillMaxWidth(), enabled = state.address.isNotBlank() && state.code.length >= 8)
        PillButton(stringResource(R.string.btn_scan_instead), { a.onManual(false) }, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet, icon = Icons.Rounded.QrCodeScanner)
    }
}

@Composable
private fun Confirm(state: PairUi.Confirm, a: PairActions) {
    val c = Pill.colors
    StateBlob(BlobState.Idle, Modifier.staggerIn(0), size = 180.dp)
    val name = state.pcName ?: state.address
    Text(stringResource(R.string.pair_confirm_title, name), Modifier.staggerIn(1), style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
    Spacer(Modifier.height(10.dp))
    Chip(state.address, Modifier.staggerIn(2), icon = Icons.Rounded.Computer)
    Spacer(Modifier.height(14.dp))
    Text(stringResource(R.string.pair_confirm_body), Modifier.staggerIn(3), style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
    Spacer(Modifier.height(24.dp))
    Column(Modifier.fillMaxWidth().staggerIn(4), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        PillButton(stringResource(R.string.btn_pair), a.onConfirm, Modifier.fillMaxWidth())
        PillButton(stringResource(R.string.btn_cancel), a.onBack, Modifier.fillMaxWidth(), kind = ButtonKind.Glass)
    }
}

@Composable
private fun Status(blob: BlobState, title: String, body: String) {
    val c = Pill.colors
    Spacer(Modifier.height(32.dp))
    StateBlob(blob, size = 220.dp)
    Spacer(Modifier.height(8.dp))
    Text(title, style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
    Spacer(Modifier.height(8.dp))
    Text(body, style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
}

@Composable
private fun Failure(state: PairUi.Failed, a: PairActions) {
    val c = Pill.colors
    Spacer(Modifier.height(16.dp))
    StateBlob(BlobState.Error, size = 190.dp)
    val (title, body) = errorTexts(state.error)
    Text(title, style = MaterialTheme.typography.headlineMedium, color = c.text, textAlign = TextAlign.Center)
    Spacer(Modifier.height(8.dp))
    Text(body, style = MaterialTheme.typography.bodyLarge, color = c.textDim, textAlign = TextAlign.Center)
    state.address?.let { Spacer(Modifier.height(12.dp)); Chip(it, icon = Icons.Rounded.Computer) }
    Spacer(Modifier.height(24.dp))
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        PillButton(stringResource(R.string.btn_scan_qr), a.onScan, Modifier.fillMaxWidth(), icon = Icons.Rounded.QrCodeScanner)
        PillButton(stringResource(R.string.btn_enter_code), { a.onManual(true) }, Modifier.fillMaxWidth(), kind = ButtonKind.Glass, icon = Icons.Rounded.Keyboard)
        if (state.error == PairError.Unreachable) {
            PillButton(stringResource(R.string.btn_try_again), a.onRetry, Modifier.fillMaxWidth(), kind = ButtonKind.Quiet, icon = Icons.Rounded.Refresh)
        }
    }
}

@Composable
private fun errorTexts(e: PairError): Pair<String, String> = when (e) {
    PairError.BadCode -> stringResource(R.string.pair_err_code_title) to stringResource(R.string.pair_err_code_body)
    PairError.Unreachable -> stringResource(R.string.pair_err_unreachable_title) to stringResource(R.string.pair_err_unreachable_body)
    PairError.InvalidLink -> stringResource(R.string.pair_err_link_title) to stringResource(R.string.pair_err_link_body)
    PairError.Refused -> stringResource(R.string.pair_err_refused_title) to stringResource(R.string.pair_err_refused_body)
}

@Composable
private fun ErrorBanner(e: PairError) {
    val (title, body) = errorTexts(e)
    Banner("$title\n$body", Icons.Rounded.ErrorOutline, tint = Pill.colors.bad)
}

@Composable
fun PillTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    hint: String,
    keyboard: KeyboardType,
    ime: ImeAction,
    modifier: Modifier = Modifier,
    mono: Boolean = false,
    capitalize: Boolean = false,
    onDone: () -> Unit = {},
) {
    val c = Pill.colors
    val source = remember { MutableInteractionSource() }
    val focused by source.collectIsFocusedAsState()
    val border by animateColorAsState(if (focused) c.accent else c.border, Motion.calm(), label = "fieldBorder")
    val shape = RoundedCornerShape(18.dp)
    Column(modifier.fillMaxWidth()) {
        Text(label, Modifier.padding(start = 4.dp, bottom = 8.dp), style = MaterialTheme.typography.labelMedium, color = c.textDim)
        BasicTextField(
            value, onValueChange,
            singleLine = true,
            interactionSource = source,
            textStyle = MaterialTheme.typography.titleMedium.copy(
                color = c.text,
                fontFamily = if (mono) FontFamily.Monospace else FontFamily.Default,
                letterSpacing = if (mono) 3.sp else TextUnit.Unspecified,
            ),
            cursorBrush = SolidColor(c.accent),
            keyboardOptions = KeyboardOptions(
                keyboardType = keyboard, imeAction = ime,
                capitalization = if (capitalize) KeyboardCapitalization.Characters else KeyboardCapitalization.None, autoCorrectEnabled = false,
            ),
            keyboardActions = KeyboardActions(onDone = { onDone() }),
            decorationBox = { inner ->
                Box(
                    Modifier.fillMaxWidth().clip(shape).background(c.background.copy(alpha = if (c.dark) 0.55f else 0.5f))
                        .border(if (focused) 1.5.dp else 1.dp, border, shape).padding(horizontal = 18.dp, vertical = 16.dp),
                ) {
                    if (value.isEmpty()) Text(hint, style = MaterialTheme.typography.titleMedium, color = c.textDim.copy(alpha = 0.6f))
                    inner()
                }
            },
        )
    }
}
