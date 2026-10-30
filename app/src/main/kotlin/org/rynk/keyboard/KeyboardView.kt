// SPDX-License-Identifier: GPL-3.0-or-later
package org.rynk.keyboard

import android.annotation.SuppressLint
import android.os.Build
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.WindowInsets

/** The keyboard surface: Rust draws it with Vulkan, this view only measures and forwards touches. */
@SuppressLint("ViewConstructor")
class KeyboardView(private val ime: RynkIme) : SurfaceView(ime), SurfaceHolder.Callback {
    private var bottomInset = 0
    /** Token of our surface in the core; 0 when none is attached */
    private var token = 0

    /** Is a drawing surface attached right now? */
    val hasSurface: Boolean get() = token != 0

    init {
        holder.addCallback(this)
        isHapticFeedbackEnabled = true
    }

    /** Space the system bars take at the bottom; the keys must stay above them. */
    private fun bottomOf(insets: WindowInsets?): Int {
        if (insets == null) return 0
        return if (Build.VERSION.SDK_INT >= 30) {
            val t = Platform.bottomInsetType
            maxOf(
                insets.getInsets(t).bottom,
                insets.getInsetsIgnoringVisibility(t).bottom,
                insets.getInsets(android.view.WindowInsets.Type.tappableElement()).bottom,
            )
        } else {
            @Suppress("DEPRECATION") insets.systemWindowInsetBottom
        }
    }

    override fun onApplyWindowInsets(insets: WindowInsets): WindowInsets {
        val bottom = bottomOf(insets)
        if (bottom != bottomInset) {
            bottomInset = bottom
            requestLayout()
        }
        return insets
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        // The input method window does not always dispatch insets to its content,
        // so read them from the root window as well.
        bottomInset = maxOf(bottomInset, bottomOf(rootWindowInsets))
        val width = MeasureSpec.getSize(widthMeasureSpec)
        val height = Native.kbMeasure(width, resources.displayMetrics.density, bottomInset, resources.displayMetrics.heightPixels)
        setMeasuredDimension(width, height)
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        token = Native.kbAttach(holder.surface)
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        if (token == 0) token = Native.kbAttach(holder.surface)
        Native.kbResize(token, width, height)
        ime.forceRender()
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        Native.kbDetach(token)
        token = 0
    }

    /** The surface keeps giving no images: attach it again (a fresh swapchain) */
    fun reattach() {
        val surface = holder.surface
        if (surface == null || !surface.isValid) return
        if (token != 0) Native.kbDetach(token)
        token = Native.kbAttach(surface)
        val r = holder.surfaceFrame
        if (token != 0) Native.kbResize(token, r.width(), r.height())
        ime.forceRender()
    }

    @SuppressLint("ClickableViewAccessibility")
    override fun onTouchEvent(e: MotionEvent): Boolean {
        Platform.forEachPointer(e) { a, id, x, y, t -> Native.kbTouch(a, id, x, y, t) }
        ime.afterEvent()
        return true
    }

    fun haptic() {
        performHapticFeedback(android.view.HapticFeedbackConstants.KEYBOARD_TAP, View.HAPTIC_FEEDBACK_ENABLED)
    }
}
