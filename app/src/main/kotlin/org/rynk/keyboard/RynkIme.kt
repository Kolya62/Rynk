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

    // The default hides the keyboard whenever Android thinks a hardware keyboard is attached;
    // Bluetooth devices (remotes, some headsets) can claim to be one, and then tapping a text
    // field showed nothing
    override fun onEvaluateInputViewShown(): Boolean = true

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        Native.kbNight(Platform.night(this))
        view?.requestLayout()
    }

    override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
        super.onStartInput(attribute, restarting)
        expected.clear()
        cursor = attribute.initialSelEnd
        composingStart = -1
        Native.kbStartInput(attribute.inputType, attribute.imeOptions, restarting)
        sendContext()
        afterEvent()
    }

    override fun onWindowShown() {
        super.onWindowShown()
        Native.kbDiag("window shown, view ${view != null}, surface ${view?.holder?.surface?.isValid}")
        // After a quick hide and show Android may not give the SurfaceView a new surface: the
        // window then shows only the navigation buttons. A layout pass usually brings it back;
        // if not, the keyboard view is created anew.
        view?.let {
            it.requestLayout()
            it.invalidate()
        }
        main.removeCallbacks(ensureSurface)
        main.postDelayed(ensureSurface, 150)
        forceRender()
    }

    private val ensureSurface = Runnable {
        val v = view
        if (isInputViewShown && v != null && !v.hasSurface) {
            Native.kbDiag("no surface 150 ms after show: recreating the keyboard view")
            setInputView(onCreateInputView())
        }
    }

    override fun onWindowHidden() {
        super.onWindowHidden()
        main.removeCallbacks(ensureSurface)
        Native.kbDiag("window hidden")
    }

    override fun onStartInputView(info: EditorInfo, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        // Let the keyboard background show through the navigation bar instead of a light scrim
        window?.window?.let { w ->
            @Suppress("DEPRECATION")
            w.navigationBarColor = android.graphics.Color.TRANSPARENT
            if (android.os.Build.VERSION.SDK_INT >= 29) w.isNavigationBarContrastEnforced = false
        }
        // a touch whose release went to a hidden window must not linger
        Native.kbTouch(3, 0, 0f, 0f, SystemClock.uptimeMillis())
        view?.requestLayout()
        forceRender()
        afterEvent()
    }

    override fun onFinishInput() {
        currentInputConnection?.finishComposingText()
        composingStart = -1
        Native.kbFinishInput()
        main.removeCallbacks(tick)
        super.onFinishInput()
    }

    /** Cursor positions our recent edits lead to, oldest first, until the editor reports them */
    private val expected = ArrayDeque<Int>()

    private fun expect(pos: Int) {
        if (pos < 0) return
        expected.addLast(pos)
        while (expected.size > 32) expected.removeFirst()
    }

    override fun onUpdateSelection(
        oldSelStart: Int, oldSelEnd: Int, newSelStart: Int, newSelEnd: Int,
        candidatesStart: Int, candidatesEnd: Int,
    ) {
        super.onUpdateSelection(oldSelStart, oldSelEnd, newSelStart, newSelEnd, candidatesStart, candidatesEnd)
        // Browsers report selections late: an update may describe a state we already moved past.
        // It is still ours when it matches a cursor we produced recently, or when the editor's
        // composing region is the one we are typing into.
        val composingOurs = composingStart >= 0 && candidatesStart == composingStart && candidatesEnd >= candidatesStart
        val ours = newSelStart == newSelEnd &&
            (newSelEnd == cursor || expected.contains(newSelEnd) || composingOurs)
        if (ours) {
            // drop expectations up to the one this update confirms
            while (expected.isNotEmpty() && expected.first() != newSelEnd) expected.removeFirst()
            if (expected.isNotEmpty()) expected.removeFirst()
        } else {
            expected.clear()
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

    /**
     * Frames are drawn on their own thread: handing a frame to the display can wait for it,
     * and touches on the main thread must never wait behind that. At most one frame is queued;
     * changes made while a frame is drawn queue the next one.
     */
    private val renderThread = android.os.HandlerThread("rynk-render", android.os.Process.THREAD_PRIORITY_DISPLAY).apply { start() }
    private val renderHandler = android.os.Handler(renderThread.looper)
    private val renderQueued = java.util.concurrent.atomic.AtomicBoolean(false)
    private var retries = 0
    private val renderFrame: Runnable = object : Runnable {
        override fun run() {
            renderQueued.set(false)
            val r = if (Native.kbNeedsRender()) Native.kbRender() else 0
            if (r == 2 && isInputViewShown) Native.kbDiag("frame skipped: no surface while shown")
            when (r) {
                // the surface had no image ready: keep trying while it exists, slower over time
                1 -> if (renderQueued.compareAndSet(false, true)) {
                    renderHandler.postDelayed(this, if (retries++ < 30) 16L else 100L)
                }
                // it keeps failing: ask the view for a fresh swapchain on the same surface
                3 -> {
                    retries = 0
                    main.post { view?.reattach() }
                }
                // drawn, or no surface (hidden): the next change or surface schedules a frame
                else -> retries = 0
            }
        }
    }

    /** Draws a frame on the render thread when something changed (never more than one queued) */
    fun scheduleRender() {
        if (Native.kbNeedsRender() && renderQueued.compareAndSet(false, true)) {
            renderHandler.post(renderFrame)
        }
    }

    /** A new or resized surface must be drawn even if nothing else changed */
    fun forceRender() {
        Native.kbInvalidate()
        scheduleRender()
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
                expect(cursor)
            }
            "T" -> ic?.let {
                val start = if (composingStart >= 0) composingStart else cursor
                it.commitText(arg, 1)
                composingStart = -1
                cursor = if (start >= 0) start + arg.length else -1
                expect(cursor)
            }
            "D" -> ic?.let {
                it.deleteSurroundingText(arg.length, 0)
                if (cursor >= 0) cursor -= arg.length
                expect(cursor)
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
