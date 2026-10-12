package org.rynk.keyboard

import android.app.AlertDialog
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.graphics.drawable.ColorDrawable
import android.inputmethodservice.InputMethodService
import android.content.res.Configuration
import android.os.Build
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup
import android.view.Window
import android.view.WindowManager
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.widget.FrameLayout
import android.widget.Toast

class RynkInputMethodService : InputMethodService() {

    companion object {
        /** Enough for the word being typed and two previous words */
        private const val EDITOR_CONTEXT_CHARS = 256
        /** Words a backspace drag can reach */
        private const val WORD_SELECT_CONTEXT_CHARS = 1000
        private const val EDITOR_SYNC_DELAY_MS = 80L
    }

    private lateinit var hapticManager: HapticManager
    private var keyboardView: RynkKeyboardView? = null
    private var clipboardListener: ClipboardManager.OnPrimaryClipChangedListener? = null
    private var isDispatchingEvents = false
    private var currentInputFieldMode: Int = NativeBridge.INPUT_MODE_NORMAL
    private var consumedClipboardText: String? = null
    private var currentEnterAction: Int = 0
    private var currentInputType: Int = 0
    /** Last known selection from onUpdateSelection */
    private var selectionStart = -1
    private var selectionEnd = -1
    /** Backspace drag: cursor position when the selection started and the text before it */
    private var wordSelectAnchor = -1
    private var wordSelectText: CharSequence = ""
    private val syncHandler = android.os.Handler(android.os.Looper.getMainLooper())
    private val delayedSync = Runnable { syncEditorContext() }
    private val clipboardHistory by lazy { ClipboardHistory(java.io.File(filesDir, ClipboardHistory.FILE_NAME)) }

    override fun onCreate() {
        super.onCreate()
        hapticManager = HapticManager(this)
        if (NativeBridge.isLibraryLoaded()) {
            NativeBridge.nativeSetAssetManager(applicationContext.assets)
        }

        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        clipboardListener = ClipboardManager.OnPrimaryClipChangedListener {
            consumedClipboardText = null
            recordClipboard()
            updateClipboardChip()
        }
        clipboard?.addPrimaryClipChangedListener(clipboardListener)
        Prefs.get(this).registerOnSharedPreferenceChangeListener(prefsListener)
    }

    override fun onDestroy() {
        saveAdaptiveDictionary()
        if (NativeBridge.isCoreInitialized) {
            NativeBridge.nativeDestroy()
            NativeBridge.isCoreInitialized = false
        }
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
        clipboardListener?.let { clipboard?.removePrimaryClipChangedListener(it) }
        Prefs.get(this).unregisterOnSharedPreferenceChangeListener(prefsListener)
        super.onDestroy()
    }

    /**
     * Called by the keyboard view right after the native core is created. Everything pushed
     * into the core before this point was dropped, so state is (re)applied here.
     */
    fun onCoreCreated() {
        UserDictionaryManager(this).syncToNative()
        loadAdaptiveDictionary()
        val prefs = Prefs.get(this)
        applySettings(prefs)
        NativeBridge.nativeSetInputFieldMode(currentInputFieldMode)
        NativeBridge.nativeSetEnterAction(currentEnterAction)
        updateClipboardChip()
        pushClipboardHistory()
        syncEditorContext()
    }

    /** Adds the new primary clip to the history unless disabled or marked sensitive. */
    private fun recordClipboard() {
        val prefs = Prefs.get(this)
        if (!prefs.getBoolean(Prefs.CLIPBOARD_HISTORY, true)) return
        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val clip = clipboard.primaryClip ?: return
        // Password managers and the system mark secrets (Android 13+); never keep them
        if (clip.description?.extras?.getBoolean("android.content.extra.IS_SENSITIVE") == true) return
        if (isSensitiveField()) return
        val text = clip.getItemAt(0)?.coerceToText(this)?.toString() ?: return
        clipboardHistory.ttlMillis = Prefs.clipboardTtlMillis(prefs)
        clipboardHistory.add(text, System.currentTimeMillis())
        pushClipboardHistory()
    }

    private fun pushClipboardHistory() {
        if (!NativeBridge.isCoreInitialized) return
        val prefs = Prefs.get(this)
        val items = if (prefs.getBoolean(Prefs.CLIPBOARD_HISTORY, true)) {
            clipboardHistory.ttlMillis = Prefs.clipboardTtlMillis(prefs)
            clipboardHistory.items(System.currentTimeMillis())
        } else {
            emptyList()
        }
        NativeBridge.nativeSetClipboardHistory(
            items.map { it.text }.toTypedArray(),
            BooleanArray(items.size) { items[it].pinned }
        )
    }

    private fun isSensitiveField(): Boolean =
        currentInputFieldMode == NativeBridge.INPUT_MODE_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_VISIBLE_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_NUMBER_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_SENSITIVE

    /**
     * Gives the engine the text before the cursor (word being typed, previous words) and the
     * editor's auto-capitalization request. Text is never read from password or incognito fields.
     */
    private fun syncEditorContext() {
        if (!NativeBridge.isCoreInitialized) return
        val ic = currentInputConnection ?: return
        val caps = ic.getCursorCapsMode(currentInputType) != 0
        val text = if (isSensitiveField()) "" else ic.getTextBeforeCursor(EDITOR_CONTEXT_CHARS, 0)?.toString() ?: ""
        NativeBridge.nativeSetEditorContext(text, caps)
        keyboardView?.onEngineStateChanged()
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        super.onFinishInputView(finishingInput)
        saveAdaptiveDictionary()
        // A finger can still be down when the keyboard hides; its release never comes
        keyboardView?.resetState()
    }

    private fun saveAdaptiveDictionary() {
        if (!NativeBridge.isLibraryLoaded()) return
        try {
            val data = NativeBridge.nativeSaveAdaptiveData() ?: return
            val targetFile = java.io.File(filesDir, "adaptive_dict.bin")
            val tempFile = java.io.File(filesDir, "adaptive_dict.bin.tmp")

            java.io.FileOutputStream(tempFile).use { fos ->
                fos.write(data)
                fos.flush()
                fos.fd.sync()
            }
            if (!tempFile.renameTo(targetFile)) {
                targetFile.delete()
                tempFile.renameTo(targetFile)
            }
        } catch (e: Exception) {
            android.util.Log.e("RynkIME", "Failed to atomically save adaptive dictionary", e)
        }
    }

    private fun loadAdaptiveDictionary() {
        if (!NativeBridge.isLibraryLoaded()) return
        try {
            val file = java.io.File(filesDir, "adaptive_dict.bin")
            if (file.exists() && file.length() > 0) {
                val bytes = file.readBytes()
                NativeBridge.nativeLoadAdaptiveData(bytes)
            }
        } catch (e: Exception) {
            android.util.Log.e("RynkIME", "Failed to load adaptive dictionary", e)
        }
    }

    private fun updateClipboardChip() {
        if (!NativeBridge.isLibraryLoaded()) return

        if (!Prefs.get(this).getBoolean(Prefs.CLIPBOARD_CHIP, true)) {
            NativeBridge.nativeSetClipboardText(null)
            keyboardView?.invalidate()
            return
        }

        // Privacy: Never read or display clipboard content when editing password or sensitive fields
        if (currentInputFieldMode == NativeBridge.INPUT_MODE_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_VISIBLE_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_NUMBER_PASSWORD ||
            currentInputFieldMode == NativeBridge.INPUT_MODE_SENSITIVE) {
            NativeBridge.nativeSetClipboardText(null)
            keyboardView?.invalidate()
            return
        }

        val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager ?: return
        val clip = clipboard.primaryClip
        if (clip != null && clip.itemCount > 0) {
            val text = clip.getItemAt(0)?.coerceToText(this)?.toString()?.trim()
            if (!text.isNullOrEmpty() && text.length <= 1000) {
                if (text == consumedClipboardText) {
                    // Single-use guarantee: already consumed or deleted, do not show again
                    NativeBridge.nativeSetClipboardText(null)
                    keyboardView?.invalidate()
                    return
                }
                NativeBridge.nativeSetClipboardText(text)
                keyboardView?.invalidate()
                return
            }
        }
        NativeBridge.nativeSetClipboardText(null)
        keyboardView?.invalidate()
    }

    private fun clearSystemClipboard() {
        try {
            val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                clipboard?.clearPrimaryClip()
            } else {
                clipboard?.setPrimaryClip(android.content.ClipData.newPlainText("", ""))
            }
        } catch (e: Exception) {
            android.util.Log.e("RynkIME", "Failed to clear clipboard", e)
        }
        consumedClipboardText = null
        NativeBridge.nativeSetClipboardText(null)
        keyboardView?.invalidate()
        Toast.makeText(this, getString(R.string.clipboard_cleared), Toast.LENGTH_SHORT).show()
    }

    override fun onConfigureWindow(win: Window, isFullscreen: Boolean, isCandidatesOnly: Boolean) {
        super.onConfigureWindow(win, isFullscreen, isCandidatesOnly)
        val prefs = Prefs.get(this)
        val themeId = Prefs.theme(prefs)
        applyWindowTheming(win, themeId)
    }

    private var inputContainerView: FrameLayout? = null

    override fun onCreateInputView(): View {
        val root = FrameLayout(this).apply {
            layoutParams = ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT
            )
        }
        inputContainerView = root

        val view = RynkKeyboardView(this).apply {
            layoutParams = FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.CENTER_HORIZONTAL or Gravity.BOTTOM
            )
        }
        keyboardView = view
        root.addView(view)

        root.setOnApplyWindowInsetsListener { _, insets ->
            view.dispatchApplyWindowInsets(insets)
            insets
        }

        val prefs = Prefs.get(this)
        val themeId = Prefs.theme(prefs)
        updateThemeAndWindowColors(themeId)
        applySettings(prefs)

        view.setOnEventsReadyListener {
            pollAndDispatchOutputEvents()
        }

        return root
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        keyboardView?.requestLayout()
        inputContainerView?.requestLayout()
        val prefs = Prefs.get(this)
        val themeId = Prefs.theme(prefs)
        updateThemeAndWindowColors(themeId)
    }

    fun determineInputFieldMode(info: EditorInfo?): Int {
        if (info == null) return NativeBridge.INPUT_MODE_NORMAL

        // Check IME_FLAG_NO_PERSONALIZED_LEARNING for incognito/private browsing mode
        val isNoPersonalizedLearning = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            (info.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING) != 0
        } else {
            false
        }

        if (isNoPersonalizedLearning) {
            return NativeBridge.INPUT_MODE_SENSITIVE
        }

        val inputType = info.inputType
        val clazz = inputType and EditorInfo.TYPE_MASK_CLASS
        val variation = inputType and EditorInfo.TYPE_MASK_VARIATION

        return when (clazz) {
            EditorInfo.TYPE_CLASS_TEXT -> {
                when (variation) {
                    EditorInfo.TYPE_TEXT_VARIATION_PASSWORD,
                    EditorInfo.TYPE_TEXT_VARIATION_WEB_PASSWORD -> NativeBridge.INPUT_MODE_PASSWORD
                    EditorInfo.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD -> NativeBridge.INPUT_MODE_VISIBLE_PASSWORD
                    EditorInfo.TYPE_TEXT_VARIATION_EMAIL_ADDRESS,
                    EditorInfo.TYPE_TEXT_VARIATION_WEB_EMAIL_ADDRESS -> NativeBridge.INPUT_MODE_EMAIL
                    EditorInfo.TYPE_TEXT_VARIATION_URI -> NativeBridge.INPUT_MODE_URI
                    else -> {
                        if ((inputType and EditorInfo.TYPE_TEXT_FLAG_MULTI_LINE) != 0) {
                            NativeBridge.INPUT_MODE_MULTILINE
                        } else {
                            NativeBridge.INPUT_MODE_NORMAL
                        }
                    }
                }
            }
            EditorInfo.TYPE_CLASS_NUMBER -> {
                if (variation == EditorInfo.TYPE_NUMBER_VARIATION_PASSWORD) {
                    NativeBridge.INPUT_MODE_NUMBER_PASSWORD
                } else {
                    NativeBridge.INPUT_MODE_NUMBER
                }
            }
            EditorInfo.TYPE_CLASS_PHONE -> NativeBridge.INPUT_MODE_PHONE
            EditorInfo.TYPE_CLASS_DATETIME -> {
                when (variation) {
                    EditorInfo.TYPE_DATETIME_VARIATION_DATE -> NativeBridge.INPUT_MODE_DATE
                    EditorInfo.TYPE_DATETIME_VARIATION_TIME -> NativeBridge.INPUT_MODE_TIME
                    else -> NativeBridge.INPUT_MODE_DATE
                }
            }
            else -> NativeBridge.INPUT_MODE_NORMAL
        }
    }

    override fun onStartInputView(info: EditorInfo?, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        keyboardView?.resetState()

        currentInputFieldMode = determineInputFieldMode(info)
        currentEnterAction = EditorSync.enterAction(info?.imeOptions ?: 0)
        currentInputType = info?.inputType ?: 0
        if (NativeBridge.isLibraryLoaded()) {
            NativeBridge.nativeSetInputFieldMode(currentInputFieldMode)
            NativeBridge.nativeSetEnterAction(currentEnterAction)
        }
        syncEditorContext()

        updateClipboardChip()

        val prefs = Prefs.get(this)
        updateThemeAndWindowColors(Prefs.theme(prefs))
        applySettings(prefs)
        // Expired entries disappear from the panel
        pushClipboardHistory()
    }

    /** Pushes every setting into the engine, the view and the feedback manager. */
    private fun applySettings(prefs: android.content.SharedPreferences) {
        hapticManager.isEnabled = prefs.getBoolean(Prefs.HAPTICS, true)
        hapticManager.strength = Prefs.hapticStrength(prefs)
        hapticManager.soundEnabled = prefs.getBoolean(Prefs.SOUND, false)
        hapticManager.soundVolume = Prefs.soundVolume(prefs) / 100f
        keyboardView?.setHeightPercent(Prefs.heightPercent(prefs))
        val numberRow = prefs.getBoolean(Prefs.NUMBER_ROW, false)
        keyboardView?.setNumberRow(numberRow)
        if (!NativeBridge.isLibraryLoaded()) return
        NativeBridge.nativeSetLayoutOptions(
            numberRow,
            Prefs.oneHanded(prefs),
            prefs.getBoolean(Prefs.VOICE_KEY, true) && voiceInputMethod() != null
        )
        NativeBridge.nativeSetEnabledLanguages(Prefs.enabledLanguages(prefs))
        NativeBridge.nativeSetProfanityEnabled(prefs.getBoolean(Prefs.PROFANITY, true))
        NativeBridge.nativeSetPopupEnabled(prefs.getBoolean(Prefs.POPUP, true))
        NativeBridge.nativeSetAdaptiveLearningEnabled(prefs.getBoolean(Prefs.ADAPTIVE_LEARNING, true))
        NativeBridge.nativeSetEngineSettings(
            Prefs.engineFlags(prefs),
            Prefs.doubleSpace(prefs),
            Prefs.autocorrectLevel(prefs)
        )
    }

    /** Settings changed (settings screen runs in this process): apply them right away. */
    private val prefsListener = android.content.SharedPreferences.OnSharedPreferenceChangeListener { prefs, key ->
        applySettings(prefs)
        when (key) {
            Prefs.THEME, Prefs.THEME_LIST -> updateThemeAndWindowColors(Prefs.theme(prefs))
            Prefs.CLIPBOARD_CHIP -> updateClipboardChip()
            Prefs.CLIPBOARD_HISTORY, Prefs.CLIPBOARD_TTL -> {
                if (!prefs.getBoolean(Prefs.CLIPBOARD_HISTORY, true)) clipboardHistory.clearAll()
                pushClipboardHistory()
            }
        }
        keyboardView?.onEngineStateChanged()
    }

    override fun onUpdateSelection(
        oldSelStart: Int,
        oldSelEnd: Int,
        newSelStart: Int,
        newSelEnd: Int,
        candidatesStart: Int,
        candidatesEnd: Int
    ) {
        super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd)
        selectionStart = newSelStart
        selectionEnd = newSelEnd
        if (isDispatchingEvents) return
        if (keyboardView?.hasActivePointers() == true) return
        if (oldSelStart == newSelStart && oldSelEnd == newSelEnd) return
        if (!NativeBridge.isCoreInitialized) return

        if (newSelStart != newSelEnd) {
            // Typing replaces the selection: continue from the text before it
            syncEditorContext()
            return
        }
        val state = NativeBridge.nativeGetComposingState() ?: return
        val composing = state.substringBefore('\t')
        val lastWord = state.substringAfter('\t', "")
        val ic = currentInputConnection ?: return
        val window = (maxOf(composing.length, lastWord.length) + 2).coerceAtMost(64)
        val textBefore = ic.getTextBeforeCursor(window, 0) ?: return
        if (!EditorSync.isEngineInSync(textBefore, composing, lastWord)) {
            syncEditorContext()
        }
    }

    fun showRemoveWordDialog(word: String) {
        val builder = AlertDialog.Builder(this)
        builder.setTitle("${getString(R.string.btn_delete)} «$word»?")
        builder.setPositiveButton(getString(R.string.btn_delete)) { _, _ ->
            UserDictionaryManager(this).removeWord(word)
            NativeBridge.nativeRemoveUserWord(word)
            hapticManager.performHaptic(3)
            keyboardView?.invalidate()
        }
        builder.setNegativeButton(getString(R.string.btn_cancel), null)

        val dialog = builder.create()
        val window = dialog.window
        if (window != null) {
            val lp = window.attributes
            lp.token = keyboardView?.windowToken
            lp.type = WindowManager.LayoutParams.TYPE_APPLICATION_ATTACHED_DIALOG
            window.attributes = lp
            try {
                dialog.show()
                hapticManager.performHaptic(3)
            } catch (e: Exception) {
                // If attached dialog fails in this context, directly remove
                UserDictionaryManager(this).removeWord(word)
                NativeBridge.nativeRemoveUserWord(word)
                hapticManager.performHaptic(3)
                keyboardView?.invalidate()
            }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (keyCode == KeyEvent.KEYCODE_BACK && event.action == KeyEvent.ACTION_DOWN) {
            if (NativeBridge.isLibraryLoaded() && NativeBridge.nativeHandleBack()) {
                keyboardView?.invalidate()
                return true
            }
        }
        return super.onKeyDown(keyCode, event)
    }

    private fun updateThemeAndWindowColors(themeId: Int) {
        val bgColor = keyboardView?.getThemeBgColor(themeId) ?: when (themeId) {
            1 -> android.graphics.Color.rgb(238, 240, 245)
            2 -> android.graphics.Color.BLACK
            3 -> android.graphics.Color.rgb(30, 22, 42)
            else -> android.graphics.Color.rgb(56, 58, 64)
        }
        inputContainerView?.setBackgroundColor(bgColor)
        keyboardView?.applyTheme(themeId)
        window?.window?.let { win ->
            applyWindowTheming(win, themeId)
        }
    }

    private fun applyWindowTheming(win: Window, themeId: Int) {
        val bgColor = keyboardView?.getThemeBgColor(themeId) ?: when (themeId) {
            1 -> android.graphics.Color.rgb(238, 240, 245) // Light #EEF0F5
            2 -> android.graphics.Color.BLACK               // AMOLED
            3 -> android.graphics.Color.rgb(30, 22, 42)     // Sunset
            else -> android.graphics.Color.rgb(56, 58, 64)  // Soft Yandex Graphite #383A40
        }

        win.setBackgroundDrawable(ColorDrawable(bgColor))
        win.navigationBarColor = bgColor

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            win.isNavigationBarContrastEnforced = false
        }

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val decor = win.decorView
            var flags = decor.systemUiVisibility
            flags = if (themeId == 1) { // Light theme needs dark navigation bar icons
                flags or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            } else {
                flags and View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR.inv()
            }
            decor.systemUiVisibility = flags
        }
    }

    private fun pollAndDispatchOutputEvents() {
        if (!NativeBridge.isLibraryLoaded()) return

        val binaryBytes = NativeBridge.nativePollEventsBinary() ?: return
        if (binaryBytes.size < 4) return

        val buffer = java.nio.ByteBuffer.wrap(binaryBytes).order(java.nio.ByteOrder.LITTLE_ENDIAN)
        val eventCount = buffer.int
        if (eventCount <= 0) return

        val ic = currentInputConnection
        isDispatchingEvents = true
        ic?.beginBatchEdit()
        try {
            for (i in 0 until eventCount) {
                if (!buffer.hasRemaining()) break
                val type = buffer.get().toInt() and 0xFF
                val len = buffer.int
                if (buffer.remaining() < len) break

                when (type) {
                    NativeBridge.EVENT_COMMIT_TEXT -> {
                        val strBytes = ByteArray(len)
                        buffer.get(strBytes)
                        val text = String(strBytes, java.nio.charset.StandardCharsets.UTF_8)
                        ic?.commitText(text, 1)
                    }
                    NativeBridge.EVENT_DELETE_SURROUNDING -> {
                        val before = buffer.int
                        val after = buffer.int
                        deleteSurroundingGraphemes(before, after)
                    }
                    NativeBridge.EVENT_SEND_KEY_EVENT -> {
                        val keyCode = buffer.int
                        ic?.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, keyCode))
                        ic?.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, keyCode))
                    }
                    NativeBridge.EVENT_PERFORM_HAPTIC -> {
                        val hapticCode = buffer.get().toInt() and 0xFF
                        hapticManager.performHaptic(hapticCode)
                    }
                    NativeBridge.EVENT_MOVE_CURSOR -> {
                        val delta = buffer.int
                        if (delta != 0) {
                            moveCursor(delta)
                            scheduleEditorSync()
                        }
                    }
                    NativeBridge.EVENT_SELECT_WORDS_BACK -> selectWordsBack(buffer.int)
                    NativeBridge.EVENT_DELETE_SELECTION -> {
                        ic?.commitText("", 1)
                        wordSelectAnchor = -1
                        scheduleEditorSync()
                    }
                    NativeBridge.EVENT_KEY_WITH_META -> {
                        val code = buffer.int
                        val meta = buffer.int
                        val now = android.os.SystemClock.uptimeMillis()
                        ic?.sendKeyEvent(KeyEvent(now, now, KeyEvent.ACTION_DOWN, code, 0, meta))
                        ic?.sendKeyEvent(KeyEvent(now, now, KeyEvent.ACTION_UP, code, 0, meta))
                        scheduleEditorSync()
                    }
                    NativeBridge.EVENT_CLIPBOARD_HISTORY_OP -> {
                        val op = buffer.int
                        val index = buffer.int
                        val now = System.currentTimeMillis()
                        when (op) {
                            1 -> clipboardHistory.togglePin(index, now)
                            2 -> clipboardHistory.delete(index, now)
                            3 -> clipboardHistory.clearUnpinned()
                        }
                        pushClipboardHistory()
                    }
                    NativeBridge.EVENT_EDITOR_COMMAND -> {
                        runEditorCommand(buffer.int)
                        scheduleEditorSync()
                    }
                    NativeBridge.EVENT_DELETE_WORD -> {
                        deletePreviousWord()
                    }
                    NativeBridge.EVENT_OPEN_SETTINGS -> {
                        val intent = Intent(this, SettingsActivity::class.java).apply {
                            flags = Intent.FLAG_ACTIVITY_NEW_TASK
                        }
                        startActivity(intent)
                    }
                    NativeBridge.EVENT_SWITCH_IME -> {
                        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
                        imm?.showInputMethodPicker()
                    }
                    NativeBridge.EVENT_HIDE_KEYBOARD -> {
                        requestHideSelf(0)
                    }
                    NativeBridge.EVENT_CLEAR_CLIPBOARD -> {
                        clearSystemClipboard()
                    }
                    NativeBridge.EVENT_CLIPBOARD_PASTED -> {
                        val strBytes = ByteArray(len)
                        buffer.get(strBytes)
                        val text = String(strBytes, java.nio.charset.StandardCharsets.UTF_8)
                        consumedClipboardText = text
                        NativeBridge.nativeSetClipboardText(null)
                        keyboardView?.invalidate()
                    }
                    NativeBridge.EVENT_VOICE_INPUT -> switchToVoiceInput()
                    NativeBridge.EVENT_ONE_HANDED_CHANGED -> {
                        val mode = buffer.int
                        Prefs.get(this).edit().putString(Prefs.ONE_HANDED, mode.toString()).apply()
                    }
                    NativeBridge.EVENT_PERFORM_EDITOR_ACTION -> {
                        val action = buffer.int
                        ic?.performEditorAction(action)
                    }
                    else -> {
                        buffer.position(buffer.position() + len)
                    }
                }
            }
        } finally {
            ic?.endBatchEdit()
            isDispatchingEvents = false
        }
    }

    /** Re-reads the editor after cursor moves and edits, once they have been applied. */
    private fun scheduleEditorSync() {
        syncHandler.removeCallbacks(delayedSync)
        syncHandler.postDelayed(delayedSync, EDITOR_SYNC_DELAY_MS)
    }

    /** Backspace drag: select the last [words] words before where the drag started (0: none). */
    private fun selectWordsBack(words: Int) {
        val ic = currentInputConnection ?: return
        if (words <= 0) {
            if (wordSelectAnchor >= 0) ic.setSelection(wordSelectAnchor, wordSelectAnchor)
            wordSelectAnchor = -1
            return
        }
        if (wordSelectAnchor < 0) {
            if (selectionEnd < 0) return
            wordSelectAnchor = selectionEnd
            wordSelectText = ic.getTextBeforeCursor(WORD_SELECT_CONTEXT_CHARS, 0) ?: ""
        }
        val length = EditorSync.wordsBackLength(wordSelectText, words)
        ic.setSelection(wordSelectAnchor - length, wordSelectAnchor)
    }

    private fun runEditorCommand(command: Int) {
        val ic = currentInputConnection ?: return
        when (command) {
            1 -> ic.performContextMenuAction(android.R.id.selectAll)
            2 -> ic.performContextMenuAction(android.R.id.copy)
            3 -> ic.performContextMenuAction(android.R.id.cut)
            4 -> ic.performContextMenuAction(android.R.id.paste)
            5, 6 -> {
                // Ctrl+Z / Ctrl+Shift+Z: understood by most text fields
                val meta = KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON or
                    (if (command == 6) KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON else 0)
                val now = android.os.SystemClock.uptimeMillis()
                ic.sendKeyEvent(KeyEvent(now, now, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_Z, 0, meta))
                ic.sendKeyEvent(KeyEvent(now, now, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_Z, 0, meta))
            }
        }
    }

    /** An enabled voice input method and its voice subtype, if any. */
    private fun voiceInputMethod(): Pair<android.view.inputmethod.InputMethodInfo, android.view.inputmethod.InputMethodSubtype>? {
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager ?: return null
        for (imi in imm.enabledInputMethodList) {
            if (imi.packageName == packageName) continue
            for (subtype in imm.getEnabledInputMethodSubtypeList(imi, true)) {
                if (subtype.mode == "voice") return imi to subtype
            }
        }
        return null
    }

    /** Hands over to the system voice keyboard (it returns to Rynk when done). */
    private fun switchToVoiceInput() {
        val (imi, subtype) = voiceInputMethod() ?: run {
            Toast.makeText(this, R.string.voice_unavailable, Toast.LENGTH_SHORT).show()
            return
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            switchInputMethod(imi.id, subtype)
        } else {
            val token = window?.window?.attributes?.token ?: return
            val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as? InputMethodManager
            @Suppress("DEPRECATION")
            imm?.setInputMethodAndSubtype(token, imi.id, subtype)
        }
    }

    private fun moveCursor(delta: Int) {
        val ic = currentInputConnection ?: return
        if (delta < 0) {
            val count = -delta
            for (i in 0 until count) {
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_LEFT))
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DPAD_LEFT))
            }
        } else if (delta > 0) {
            for (i in 0 until delta) {
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DPAD_RIGHT))
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DPAD_RIGHT))
            }
        }
    }

    private fun deletePreviousWord() {
        val ic = currentInputConnection ?: return
        val textBefore = ic.getTextBeforeCursor(100, 0)?.toString() ?: ""
        if (textBefore.isEmpty()) {
            ic.deleteSurroundingText(1, 0)
            return
        }

        var i = textBefore.length - 1
        // Skip trailing spaces or punctuation
        while (i >= 0 && (textBefore[i].isWhitespace() || !textBefore[i].isLetterOrDigit())) {
            i--
        }
        // Skip letters of the word
        while (i >= 0 && textBefore[i].isLetterOrDigit()) {
            i--
        }

        val deleteCount = (textBefore.length - 1 - i).coerceAtLeast(1)
        ic.deleteSurroundingText(deleteCount, 0)
    }

    private fun deleteSurroundingGraphemes(before: Int, after: Int) {
        val ic = currentInputConnection ?: return
        if (before <= 0 && after <= 0) return

        if (before == 1 && after == 0) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                val textBefore = ic.getTextBeforeCursor(16, 0)?.toString()
                if (!textBefore.isNullOrEmpty()) {
                    val it = android.icu.text.BreakIterator.getCharacterInstance()
                    it.setText(textBefore)
                    val last = it.last()
                    val prev = it.previous()
                    if (prev != android.icu.text.BreakIterator.DONE) {
                        val codeUnitsToDelete = last - prev
                        if (codeUnitsToDelete > 1) {
                            var res = ic.deleteSurroundingText(codeUnitsToDelete, 0)
                            if (!res) {
                                for (k in 0 until codeUnitsToDelete) {
                                    ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL))
                                    ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL))
                                }
                            }
                            return
                        }
                    }
                }
            }

            var res = ic.deleteSurroundingText(1, 0)
            if (!res) {
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL))
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL))
            }
            return
        }

        var res = ic.deleteSurroundingText(before, after)
        if (!res && before > 0) {
            for (k in 0 until before) {
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_DEL))
                ic.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_DEL))
            }
        }
    }
}
