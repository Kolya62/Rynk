// SPDX-License-Identifier: GPL-3.0-or-later
package org.rynk.keyboard

import android.annotation.SuppressLint
import android.app.Activity
import android.content.Intent
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Choreographer
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.WindowInsets
import android.view.inputmethod.InputMethodManager
import android.window.OnBackInvokedDispatcher

/** Settings and setup screen; the whole UI is drawn by egui in Rust. */
class SettingsActivity : Activity(), SurfaceHolder.Callback, Choreographer.FrameCallback {
    private lateinit var surface: SurfaceView
    private var attached = false
    private var frameQueued = false
    private var insetTop = 0
    private var insetBottom = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        Native.stInit(filesDir.absolutePath, Platform.locales(this), Platform.night(this))
        if (Build.VERSION.SDK_INT >= 30) window.setDecorFitsSystemWindows(false)
        surface = object : SurfaceView(this) {
            @SuppressLint("ClickableViewAccessibility")
            override fun onTouchEvent(e: MotionEvent): Boolean {
                Platform.forEachPointer(e) { a, id, x, y, t -> Native.stTouch(a, id, x, y, t) }
                requestFrame()
                return true
            }

            override fun onApplyWindowInsets(insets: WindowInsets): WindowInsets {
                if (Build.VERSION.SDK_INT >= 30) {
                    val bars = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout())
                    insetTop = bars.top
                    insetBottom = bars.bottom
                } else {
                    @Suppress("DEPRECATION")
                    run { insetTop = insets.systemWindowInsetTop; insetBottom = insets.systemWindowInsetBottom }
                }
                resize()
                return insets
            }
        }
        surface.holder.addCallback(this)
        setContentView(surface)
        if (Build.VERSION.SDK_INT >= 33) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT) { back() }
        }
    }

    private fun back() {
        if (Native.stBack()) requestFrame() else finish()
    }

    @Deprecated("Used below Android 13")
    override fun onBackPressed() = back()

    override fun onResume() {
        super.onResume()
        val imm = getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager
        val enabled = imm.enabledInputMethodList.any { it.packageName == packageName }
        val current = Settings.Secure.getString(contentResolver, Settings.Secure.DEFAULT_INPUT_METHOD) ?: ""
        Native.stImeState(enabled, current.startsWith("$packageName/"), Platform.night(this))
        requestFrame()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        // returning from the keyboard picker: the selection may have changed
        if (hasFocus) onResume()
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        onResume()
    }

    private fun resize() {
        if (attached) {
            Native.stResize(surface.width, surface.height, resources.displayMetrics.density, insetTop, insetBottom)
            requestFrame()
        }
    }

    override fun surfaceCreated(holder: SurfaceHolder) {
        attached = Native.stAttach(holder.surface)
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        if (attached) {
            Native.stResize(width, height, resources.displayMetrics.density, insetTop, insetBottom)
            requestFrame()
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        attached = false
        Native.stDetach()
    }

    private fun requestFrame() {
        if (!frameQueued) {
            frameQueued = true
            Choreographer.getInstance().postFrameCallback(this)
        }
    }

    override fun doFrame(frameTimeNanos: Long) {
        frameQueued = false
        if (!attached) return
        val again = Native.stRender()
        for (c in Native.stTakeCommands()) {
            when (c) {
                'E' -> startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS))
                'P' -> (getSystemService(INPUT_METHOD_SERVICE) as InputMethodManager).showInputMethodPicker()
                'F' -> finish()
            }
        }
        if (again) requestFrame()
    }
}
