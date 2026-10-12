package org.rynk.keyboard

import android.content.Context
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Typeface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import android.view.WindowInsets

class RynkKeyboardView @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0
) : View(context, attrs, defStyleAttr) {

    private var frontBitmap: Bitmap? = null
    private var backBitmap: Bitmap? = null
    private var onEventsReadyListener: (() -> Unit)? = null
    private var bottomInset: Int = 0

    private val gestureHandler = Handler(Looper.getMainLooper())

    var currentThemeId: Int = 1
        private set

    // Pre-allocated text rendering paints for maximum 60/120fps performance
    private val textPaintRegular = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        textAlign = Paint.Align.CENTER
        typeface = Typeface.create("sans-serif", Typeface.NORMAL)
    }

    private val textPaintMedium = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        textAlign = Paint.Align.CENTER
        typeface = Typeface.create("sans-serif-medium", Typeface.NORMAL)
    }

    private val textPaintBold = Paint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
        textAlign = Paint.Align.CENTER
        typeface = Typeface.create("sans-serif", Typeface.BOLD)
    }

    fun setOnEventsReadyListener(listener: () -> Unit) {
        this.onEventsReadyListener = listener
    }

    fun getThemeBgColor(themeId: Int): Int {
        return when (themeId) {
            1 -> Color.rgb(238, 240, 245) // Light theme #EEF0F5
            2 -> Color.BLACK              // AMOLED
            3 -> Color.rgb(30, 22, 42)    // Sunset
            else -> Color.rgb(56, 58, 64) // Soft Yandex Graphite #383A40
        }
    }

    override fun onApplyWindowInsets(insets: WindowInsets): WindowInsets {
        val navInset = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            insets.getInsets(WindowInsets.Type.navigationBars()).bottom
        } else {
            @Suppress("DEPRECATION")
            insets.systemWindowInsetBottom
        }
        if (bottomInset != navInset) {
            bottomInset = navInset
            requestLayout()
        }
        return super.onApplyWindowInsets(insets)
    }

    private var isEmojiMode: Boolean = false

    /** Keyboard height as a percentage of the default (settings: 80..130) */
    private var heightPercent: Int = Prefs.heightPercent(Prefs.get(context))

    /** The number row adds a row: the keyboard grows instead of squeezing the letters */
    private var numberRow: Boolean = Prefs.get(context).getBoolean(Prefs.NUMBER_ROW, false)

    fun setNumberRow(enabled: Boolean) {
        if (enabled != numberRow) {
            numberRow = enabled
            requestLayout()
        }
    }

    fun setHeightPercent(percent: Int) {
        if (percent != heightPercent) {
            heightPercent = percent
            requestLayout()
        }
    }

    fun checkModeChange() {
        if (!NativeBridge.isLibraryLoaded()) return
        val mode = NativeBridge.nativeGetMode()
        val emojiActive = (mode == 3)
        if (isEmojiMode != emojiActive) {
            isEmojiMode = emojiActive
            requestLayout()
        }
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val density = resources.displayMetrics.density
        val config = resources.configuration
        val isLandscape = config.orientation == Configuration.ORIENTATION_LANDSCAPE
        val isTablet = config.smallestScreenWidthDp >= 600

        val screenWidthPx = MeasureSpec.getSize(widthMeasureSpec)
        val maxWidthDp = if (isTablet) {
            if (isLandscape) 900f else 720f
        } else {
            Float.MAX_VALUE
        }
        val maxWidthPx = if (maxWidthDp < Float.MAX_VALUE) (maxWidthDp * density).toInt() else Int.MAX_VALUE
        val width = if (maxWidthPx in 1 until screenWidthPx) maxWidthPx else screenWidthPx

        val desiredHeightDp = if (isTablet) {
            if (isLandscape) {
                if (isEmojiMode) 360f else 280f
            } else {
                if (isEmojiMode) 400f else 330f
            }
        } else {
            if (isLandscape) 200f else if (isEmojiMode) 350f else 280f
        }
        val rowFactor = if (numberRow && !isEmojiMode) NUMBER_ROW_HEIGHT_FACTOR else 1f
        val contentHeight = (desiredHeightDp * heightPercent / 100f * rowFactor * density).toInt()
        val totalHeight = contentHeight + bottomInset

        setMeasuredDimension(width, totalHeight)
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        if (w <= 0 || h <= 0) return

        if (!NativeBridge.isLibraryLoaded()) return

        val contentH = (h - bottomInset).coerceAtLeast(1)

        ensureBitmaps(w, contentH)

        val density = resources.displayMetrics.density
        val prefs = Prefs.get(context)
        val themeId = Prefs.theme(prefs)
        currentThemeId = themeId
        setBackgroundColor(getThemeBgColor(themeId))

        if (!NativeBridge.isCoreInitialized) {
            NativeBridge.nativeInit(w.toFloat(), contentH.toFloat(), density, themeId)
            NativeBridge.isCoreInitialized = true
            (context as? RynkInputMethodService)?.onCoreCreated()
        } else {
            NativeBridge.nativeResize(w.toFloat(), contentH.toFloat(), density)
            NativeBridge.nativeSetTheme(themeId)
        }
    }

    private fun ensureBitmaps(w: Int, contentH: Int) {
        if (frontBitmap == null || backBitmap == null || frontBitmap?.width != w || frontBitmap?.height != contentH) {
            frontBitmap?.recycle()
            backBitmap?.recycle()
            frontBitmap = Bitmap.createBitmap(w, contentH, Bitmap.Config.ARGB_8888)
            backBitmap = Bitmap.createBitmap(w, contentH, Bitmap.Config.ARGB_8888)
        }
    }

    private class ActivePointer(
        val pointerId: Int,
        val startX: Float,
        val startY: Float,
        val startTime: Long,
        val isBackspace: Boolean,
        var isRepeatingBackspace: Boolean = false,
        var isLongPressTriggered: Boolean = false,
        var longPressRunnable: Runnable? = null,
        var repeatRunnable: Runnable? = null
    )

    private val activePointers = android.util.SparseArray<ActivePointer>()

    private fun cancelAllActivePointers() {
        for (i in 0 until activePointers.size()) {
            val ptr = activePointers.valueAt(i)
            ptr.longPressRunnable?.let { gestureHandler.removeCallbacks(it) }
            ptr.repeatRunnable?.let { gestureHandler.removeCallbacks(it) }
        }
        activePointers.clear()
    }

    private fun handleSuggestionBarLongPress(x: Float, y: Float) {
        val word = NativeBridge.nativeGetSuggestionAt(x, y)
        if (word.isEmpty() || word.contains("...") || SvgIcons.isIcon(word)) {
            return
        }

        (context as? RynkInputMethodService)?.showRemoveWordDialog(word)
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (!NativeBridge.isLibraryLoaded()) return super.onTouchEvent(event)

        val contentH = (height - bottomInset).toFloat()
        val actionMasked = event.actionMasked
        val timeMs = event.eventTime

        // Multitouch: Iterate over all active pointers on ACTION_MOVE
        if (actionMasked == MotionEvent.ACTION_MOVE) {
            var anyHandled = false
            for (i in 0 until event.pointerCount) {
                val pid = event.getPointerId(i)
                val px = event.getX(i)
                val py = event.getY(i)
                if (py > contentH) continue

                val ptr = activePointers.get(pid)
                if (ptr != null) {
                    val distSq = (px - ptr.startX) * (px - ptr.startX) + (py - ptr.startY) * (py - ptr.startY)
                    if (distSq > 500f && !ptr.isLongPressTriggered) {
                        ptr.longPressRunnable?.let { gestureHandler.removeCallbacks(it) }
                    }
                    if (ptr.isBackspace) {
                        val d = resources.displayMetrics.density
                        // Dragging left selects words (or the finger slid away): stop repeat deletion
                        val dragging = (px < ptr.startX - 20f * d) || (Math.abs(py - ptr.startY) > 40f * d)
                        if (dragging) {
                            ptr.repeatRunnable?.let { gestureHandler.removeCallbacks(it) }
                        }
                    }
                }

                val h = NativeBridge.nativeOnTouchEvent(2 /* Move */, pid, px, py, timeMs)
                if (h) anyHandled = true
            }
            checkModeChange()
            onEventsReadyListener?.invoke()
            invalidate()
            return anyHandled || true
        }

        // Multitouch: Handle ACTION_CANCEL cleanly for all active touches
        if (actionMasked == MotionEvent.ACTION_CANCEL) {
            cancelAllActivePointers()
            for (i in 0 until event.pointerCount) {
                val pid = event.getPointerId(i)
                val px = event.getX(i)
                val py = event.getY(i)
                NativeBridge.nativeOnTouchEvent(3 /* Cancel */, pid, px, py, timeMs)
            }
            checkModeChange()
            onEventsReadyListener?.invoke()
            invalidate()
            return true
        }

        val actionIndex = event.actionIndex
        val pointerId = event.getPointerId(actionIndex)
        val x = event.getX(actionIndex)
        val y = event.getY(actionIndex)

        if (actionMasked == MotionEvent.ACTION_DOWN) {
            // First finger: nothing else can be held, so any leftover touch state is stale
            cancelAllActivePointers()
            NativeBridge.nativeCancelTouches()
        }

        if (y > contentH) {
            val ptr = activePointers.get(pointerId)
            if (ptr != null) {
                ptr.longPressRunnable?.let { gestureHandler.removeCallbacks(it) }
                ptr.repeatRunnable?.let { gestureHandler.removeCallbacks(it) }
                activePointers.remove(pointerId)
            }
            // Lifted over the navigation bar inset: the core still has to release the key
            if (actionMasked == MotionEvent.ACTION_UP || actionMasked == MotionEvent.ACTION_POINTER_UP) {
                NativeBridge.nativeOnTouchEvent(3 /* Cancel */, pointerId, x, y, timeMs)
                invalidate()
            }
            return false
        }

        // Handle continuous backspace hold and repeat per pointer
        if (actionMasked == MotionEvent.ACTION_DOWN || actionMasked == MotionEvent.ACTION_POINTER_DOWN) {
            val isBs = NativeBridge.nativeIsBackspaceAt(x, y)
            val p = ActivePointer(
                pointerId = pointerId,
                startX = x,
                startY = y,
                startTime = SystemClock.uptimeMillis(),
                isBackspace = isBs
            )
            activePointers.put(pointerId, p)

            val lpRunnable = Runnable {
                if (!NativeBridge.isLibraryLoaded()) return@Runnable
                val ptr = activePointers.get(pointerId) ?: return@Runnable
                ptr.isLongPressTriggered = true
                val now = SystemClock.uptimeMillis()

                val dp = resources.displayMetrics.density
                val suggestionBarH = 44f * dp
                if (ptr.startY < suggestionBarH) {
                    handleSuggestionBarLongPress(ptr.startX, ptr.startY)
                    return@Runnable
                }

                NativeBridge.nativeTick(now)
                onEventsReadyListener?.invoke()
                invalidate()
            }
            p.longPressRunnable = lpRunnable
            gestureHandler.postDelayed(lpRunnable, 350L)

            if (isBs) {
                val repRunnable = object : Runnable {
                    override fun run() {
                        if (!NativeBridge.isLibraryLoaded()) return
                        val ptr = activePointers.get(pointerId) ?: return
                        ptr.isRepeatingBackspace = true
                        val heldDuration = SystemClock.uptimeMillis() - ptr.startTime

                        val (count, delayMs) = when {
                            heldDuration > 2500L -> 2 to 35L
                            heldDuration > 1200L -> 1 to 45L
                            else -> 1 to 60L
                        }

                        NativeBridge.nativeRepeatBackspace(count)
                        onEventsReadyListener?.invoke()
                        invalidate()

                        gestureHandler.postDelayed(this, delayMs)
                    }
                }
                p.repeatRunnable = repRunnable
                gestureHandler.postDelayed(repRunnable, 350L)
            }
        } else if (actionMasked == MotionEvent.ACTION_UP || actionMasked == MotionEvent.ACTION_POINTER_UP) {
            val ptr = activePointers.get(pointerId)
            var wasRepeating = false
            if (ptr != null) {
                ptr.longPressRunnable?.let { gestureHandler.removeCallbacks(it) }
                ptr.repeatRunnable?.let { gestureHandler.removeCallbacks(it) }
                wasRepeating = ptr.isRepeatingBackspace
                activePointers.remove(pointerId)
            }

            if (wasRepeating) {
                // Suppress extra release delete after repeating
                NativeBridge.nativeOnTouchEvent(3 /* Cancel */, pointerId, x, y, timeMs)
                checkModeChange()
                onEventsReadyListener?.invoke()
                invalidate()
                return true
            }
        }

        val rustAction = when (actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> 0
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> 1
            else -> return super.onTouchEvent(event)
        }

        val handled = NativeBridge.nativeOnTouchEvent(rustAction, pointerId, x, y, timeMs)

        checkModeChange()

        // Poll keyboard output events for the InputMethodService
        onEventsReadyListener?.invoke()

        invalidate()
        return handled
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)

        // 1. Fill entire canvas (including navigation bar area) with theme background
        val bgColor = getThemeBgColor(currentThemeId)
        canvas.drawColor(bgColor)

        if (!NativeBridge.isCoreInitialized) return
        // Bitmaps are released on detach; a re-attached view of the same size gets no onSizeChanged
        if (width > 0 && height - bottomInset > 0) ensureBitmaps(width, height - bottomInset)
        val targetBmp = backBitmap ?: return

        // 2. Double-buffered Rust rendering: render into backBitmap then swap
        val nowMs = SystemClock.uptimeMillis()
        val hasActiveAnimation = NativeBridge.nativeRender(targetBmp, nowMs)

        val temp = frontBitmap
        frontBitmap = backBitmap
        backBitmap = temp

        val readyBmp = frontBitmap ?: return
        canvas.drawBitmap(readyBmp, 0f, 0f, null)

        // 3. Draw text labels on top using native Android system font (including color emojis)
        drawSystemTextLabels(canvas)

        if (hasActiveAnimation) {
            postInvalidateOnAnimation()
        }
    }

    private var cachedLabelsVersion: Long = -1L
    private var cachedLabels: List<TextLabelItem> = emptyList()

    private class TextLabelItem(
        val text: String,
        val cx: Float,
        val cy: Float,
        val fontSize: Float,
        val color: Int,
        val isBold: Boolean,
        val labelType: Int
    )

    private fun drawSystemTextLabels(canvas: Canvas) {
        val version = NativeBridge.nativeGetLabelsVersion()
        if (version != cachedLabelsVersion) {
            val labelsString = NativeBridge.nativeGetTextLabels()
            if (labelsString.isNotEmpty()) {
                val lines = labelsString.split("\n")
                val list = ArrayList<TextLabelItem>(lines.size)
                for (line in lines) {
                    val trimmed = line.trim()
                    if (trimmed.isEmpty()) continue

                    val parts = trimmed.split("\t")
                    if (parts.size < 10) continue

                    val text = parts[0]
                    val cx = parts[1].toFloatOrNull() ?: continue
                    val cy = parts[2].toFloatOrNull() ?: continue
                    val fontSize = parts[3].toFloatOrNull() ?: continue
                    val r = parts[4].toIntOrNull() ?: continue
                    val g = parts[5].toIntOrNull() ?: continue
                    val b = parts[6].toIntOrNull() ?: continue
                    val a = parts[7].toIntOrNull() ?: continue
                    val isBold = parts[8] == "1"
                    val labelType = parts[9].toIntOrNull() ?: 0

                    list.add(
                        TextLabelItem(
                            text = text,
                            cx = cx,
                            cy = cy,
                            fontSize = fontSize,
                            color = Color.argb(a, r, g, b),
                            isBold = isBold,
                            labelType = labelType
                        )
                    )
                }
                cachedLabels = list
            } else {
                cachedLabels = emptyList()
            }
            cachedLabelsVersion = version
        }

        val dp = resources.displayMetrics.density
        val tab_bar_h = 40f * dp
        val bottom_bar_h = 40f * dp
        val elevation_h = 36f * dp
        val bot_y = (height - bottomInset).toFloat() - elevation_h - bottom_bar_h

        for (item in cachedLabels) {
            // Clip emoji labels: in normal emoji mode they must be in grid area (below tab bar, above bottom bar)
            // In search mode the strip emojis (labelType==6) can be in header area, so we only clip
            // those below the keyboard content area (navigation zone)
            if (item.labelType == 6 && item.cy > bot_y) {
                continue
            }

            // Check if this is an SVG vector icon (Shift, Backspace, Enter, Globe, Emoji, Settings, Back)
            if (item.labelType != 6 && (item.labelType == 5 || SvgIcons.isIcon(item.text))) {
                val iconSize = (item.fontSize * 1.25f).coerceAtLeast(18f)
                val drawn = SvgIcons.draw(canvas, item.text, item.cx, item.cy, iconSize, item.color)
                if (drawn) {
                    continue
                }
            }

            val paint = when {
                item.isBold -> textPaintBold
                item.labelType == 0 -> textPaintRegular   // Character keys
                item.labelType == 6 -> textPaintRegular   // Grid emojis
                item.labelType == 1 -> textPaintRegular   // Secondary sub-labels
                item.labelType == 2 -> textPaintMedium    // Suggestion chips & shortcuts
                item.labelType == 3 -> textPaintBold      // Popups
                item.labelType == 4 -> textPaintMedium    // Brand / Space language
                else -> textPaintRegular
            }

            paint.textSize = item.fontSize
            paint.color = item.color

            val fontMetrics = paint.fontMetrics
            val baselineY = item.cy - (fontMetrics.ascent + fontMetrics.descent) / 2f

            canvas.drawText(item.text, item.cx, baselineY, paint)
        }
    }

    fun applyTheme(themeId: Int) {
        currentThemeId = themeId
        setBackgroundColor(getThemeBgColor(themeId))
        cachedLabelsVersion = -1L
        if (NativeBridge.isLibraryLoaded()) {
            NativeBridge.nativeSetTheme(themeId)
            invalidate()
        }
    }

    fun applyLanguage(langId: Int) {
        cachedLabelsVersion = -1L
        if (NativeBridge.isLibraryLoaded()) {
            NativeBridge.nativeSetLanguage(langId)
            invalidate()
        }
    }

    fun hasActivePointers(): Boolean {
        return activePointers.size() > 0
    }

    fun resetComposingState() {
        cachedLabelsVersion = -1L
        if (NativeBridge.isLibraryLoaded()) {
            NativeBridge.nativeReset()
            checkModeChange()
            invalidate()
        }
    }

    /** Redraws after the engine state changed outside a touch (editor context, cursor move). */
    fun onEngineStateChanged() {
        cachedLabelsVersion = -1L
        checkModeChange()
        invalidate()
    }

    fun resetState() {
        cancelAllActivePointers()
        if (NativeBridge.isCoreInitialized) NativeBridge.nativeCancelTouches()
        resetComposingState()
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        cancelAllActivePointers()
        frontBitmap?.recycle()
        frontBitmap = null
        backBitmap?.recycle()
        backBitmap = null
        // The native core is owned by RynkInputMethodService and outlives this view
    }

    companion object {
        /** Four letter rows become five: about a fifth more height keeps key size */
        private const val NUMBER_ROW_HEIGHT_FACTOR = 1.18f
    }
}
