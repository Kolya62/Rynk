// SPDX-License-Identifier: GPL-3.0-or-later
package org.rynk.keyboard

import android.content.Intent
import android.content.res.Configuration
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import java.util.concurrent.Executors

/**
 * Input method service. The Rust core decides what to type; this class applies its commands to
 * the editor through [android.view.inputmethod.InputConnection] and does the other system calls.
 */
class RynkIme : InputMethodService() {
    private var view: KeyboardView? = null
    private val main = Handler(Looper.getMainLooper())
    private val loader = Executors.newSingleThreadExecutor()
    private val loading = HashSet<String>()

    /** Where we expect the cursor after our own edits (UTF-16 offsets); -1 when unknown */
    private var cursor = -1
    private var composingStart = -1
    private val tick = Runnable {
        Native.kbTick(SystemClock.uptimeMillis())
        afterEvent()
    }

    override fun onCreate() {
        super.onCreate()
        Native.kbInit(filesDir.absolutePath, Platform.locales(this), Platform.night(this))
    }

    override fun onCreateInputView(): View {
        val v = KeyboardView(this)
        view = v
        return v
    }

    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        Native.kbNight(Platform.night(this))
        view?.requestLayout()
    }

    override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
        super.onStartInput(attribute, restarting)
        cursor = attribute.initialSelEnd
        composingStart = -1
        Native.kbStartInput(attribute.inputType, attribute.imeOptions, restarting)
        sendContext()
        afterEvent()
    }

    override fun onStartInputView(info: EditorInfo, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // Let the keyboard background show through the navigation bar instead of a light scrim
        window?.window?.let { w ->
            @Suppress("DEPRECATION")
            w.navigationBarColor = android.graphics.Color.TRANSPARENT
            if (android.os.Build.VERSION.SDK_INT >= 29) w.isNavigationBarContrastEnforced = false
        }
        view?.requestLayout()
        afterEvent()
    }

    override fun onFinishInput() {
        currentInputConnection?.finishComposingText()
        composingStart = -1
        Native.kbFinishInput()
        main.removeCallbacks(tick)
        super.onFinishInput()
    }

    override fun onUpdateSelection(
        oldSelStart: Int, oldSelEnd: Int, newSelStart: Int, newSelEnd: Int,
        candidatesStart: Int, candidatesEnd: Int,
    ) {
        super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd)
        val ours = newSelStart == newSelEnd && newSelEnd == cursor
        if (!ours) {
            // the user moved the cursor or the app changed the text
            cursor = newSelEnd
            if (composingStart >= 0) {
                currentInputConnection?.finishComposingText()
                composingStart = -1
                Native.kbAbandon()
            }
        }
        if (composingStart < 0) sendContext()
        afterEvent()
    }

    private fun sendContext() {
        val ic = currentInputConnection ?: return
        val before = ic.getTextBeforeCursor(64, 0)?.toString() ?: ""
        val caps = ic.getCursorCapsMode(currentInputEditorInfo?.inputType ?: 0) != 0
        Native.kbContext(before, caps)
    }

    /** Applies the core's commands, redraws if needed and schedules the next timer */
    /**
     * Frames are drawn on their own thread: handing a frame to the display can wait for it,
     * and touches on the main thread must never wait behind that. At most one frame is queued;
     * changes made while a frame is drawn queue the next one.
     */
    private val renderThread = android.os.HandlerThread("rynk-render", android.os.Process.THREAD_PRIORITY_DISPLAY).apply { start() }
    private val renderHandler = android.os.Handler(renderThread.looper)
    private val renderQueued = java.util.concurrent.atomic.AtomicBoolean(false)
    private val renderFrame = Runnable {
        renderQueued.set(false)
        if (Native.kbNeedsRender()) Native.kbRender()
        // a stale surface skips a frame; try again
        scheduleRender()
    }

    private fun scheduleRender() {
        if (Native.kbNeedsRender() && renderQueued.compareAndSet(false, true)) {
            renderHandler.post(renderFrame)
        }
    }

    fun afterEvent() {
        apply(Native.kbTakeCommands())
        scheduleRender()
        main.removeCallbacks(tick)
        val wake = Native.kbWake()
        if (wake >= 0) main.postAtTime(tick, wake)
    }

    private fun apply(cmds: String) {
        if (cmds.isEmpty()) return
        val ic = currentInputConnection
        ic?.beginBatchEdit()
        try {
            for (rec in cmds.split('\u001E')) {
                if (rec.isEmpty()) continue
                val sep = rec.indexOf('\u001F')
                val op = rec.substring(0, sep)
                val arg = rec.substring(sep + 1)
                run(op, arg, ic)
            }
        } finally {
            ic?.endBatchEdit()
        }
    }

    private fun run(op: String, arg: String, ic: android.view.inputmethod.InputConnection?) {
        when (op) {
            "C" -> ic?.let {
                if (composingStart < 0) composingStart = cursor
                it.setComposingText(arg, 1)
                if (arg.isEmpty()) {
                    cursor = composingStart
                    composingStart = -1
                } else if (composingStart >= 0) {
                    cursor = composingStart + arg.length
                }
            }
            "T" -> ic?.let {
                val start = if (composingStart >= 0) composingStart else cursor
                it.commitText(arg, 1)
                composingStart = -1
                cursor = if (start >= 0) start + arg.length else -1
            }
            "D" -> ic?.let {
                it.deleteSurroundingText(arg.length, 0)
                if (cursor >= 0) cursor -= arg.length
            }
            "K" -> {
                cursor = -1
                sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL)
            }
            "A" -> {
                val action = (currentInputEditorInfo?.imeOptions ?: 0) and EditorInfo.IME_MASK_ACTION
                if (action > EditorInfo.IME_ACTION_NONE) ic?.performEditorAction(action)
                else sendDownUpKeyEvents(KeyEvent.KEYCODE_ENTER)
            }
            "M" -> {
                val n = arg.toIntOrNull() ?: 0
                cursor = -1
                val key = if (n < 0) KeyEvent.KEYCODE_DPAD_LEFT else KeyEvent.KEYCODE_DPAD_RIGHT
                repeat(kotlin.math.abs(n)) { sendDownUpKeyEvents(key) }
            }
            "H" -> view?.haptic()
            "P" -> {
                cursor = -1
                ic?.performContextMenuAction(android.R.id.paste)
            }
            "S" -> startActivity(Intent(this, SettingsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            "I" -> (getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager).showInputMethodPicker()
            "X" -> requestHideSelf(0)
            "R" -> view?.requestLayout()
            "L" -> loadModel(arg)
            "E" -> renderEmoji(arg)
        }
    }

    /**
     * Draws emoji with the system color font, which egui cannot render itself.
     * [arg] is the image size in px followed by the emoji, space-separated.
     */
    private fun renderEmoji(arg: String) {
        val parts = arg.split(' ').filter { it.isNotEmpty() }
        val size = parts.firstOrNull()?.toIntOrNull()?.coerceIn(16, 256) ?: return
        val list = parts.drop(1)
        loader.execute {
            val paint = android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG).apply {
                textAlign = android.graphics.Paint.Align.CENTER
            }
            val bmp = android.graphics.Bitmap.createBitmap(size, size, android.graphics.Bitmap.Config.ARGB_8888)
            val canvas = android.graphics.Canvas(bmp)
            val one = size * size * 4
            // send in chunks so the first rows appear quickly
            for (chunk in list.chunked(24)) {
                val data = ByteArray(one * chunk.size)
                chunk.forEachIndexed { i, e ->
                    bmp.eraseColor(android.graphics.Color.TRANSPARENT)
                    paint.textSize = size.toFloat()
                    val w = paint.measureText(e)
                    if (w > size) paint.textSize = size * size / w
                    val fm = paint.fontMetrics
                    canvas.drawText(e, size / 2f, size / 2f - (fm.ascent + fm.descent) / 2f, paint)
                    val buf = java.nio.ByteBuffer.wrap(data, i * one, one)
                    bmp.copyPixelsToBuffer(buf)
                }
                Native.kbEmoji(chunk.joinToString(" "), size, data)
                main.post { afterEvent() }
            }
            bmp.recycle()
        }
    }

    /** Language models and dictionaries (asset paths) are decoded on a background thread */
    private fun loadModel(path: String) {
        if (!loading.add(path)) return
        loader.execute {
            try {
                val bytes = assets.open(path).use { it.readBytes() }
                Native.kbModel(path, bytes)
            } catch (e: java.io.IOException) {
                android.util.Log.w("Rynk", "missing asset $path")
            }
            main.post {
                loading.remove(path)
                afterEvent()
            }
        }
    }

    override fun onDestroy() {
        Native.kbFinishInput()
        loader.shutdown()
        renderThread.quitSafely()
        super.onDestroy()
    }
}
