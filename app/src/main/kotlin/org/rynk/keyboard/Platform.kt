// SPDX-License-Identifier: GPL-3.0-or-later
package org.rynk.keyboard

import android.content.Context
import android.content.res.Configuration
import android.os.Build
import android.view.MotionEvent

/** System facts the core needs */
internal object Platform {
    fun locales(ctx: Context): String {
        val list = ctx.resources.configuration.locales
        return (0 until list.size()).joinToString(",") { list[it].toLanguageTag() }
    }

    fun night(ctx: Context): Boolean =
        (ctx.resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES

    /** Forwards a MotionEvent as per-pointer down(0)/move(1)/up(2)/cancel(3) calls */
    inline fun forEachPointer(e: MotionEvent, send: (action: Int, id: Int, x: Float, y: Float, t: Long) -> Unit) {
        val t = e.eventTime
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                val i = e.actionIndex
                send(0, e.getPointerId(i), e.getX(i), e.getY(i), t)
            }
            MotionEvent.ACTION_MOVE -> for (i in 0 until e.pointerCount) {
                send(1, e.getPointerId(i), e.getX(i), e.getY(i), t)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                val i = e.actionIndex
                send(2, e.getPointerId(i), e.getX(i), e.getY(i), t)
            }
            MotionEvent.ACTION_CANCEL -> send(3, 0, 0f, 0f, t)
        }
    }

    val bottomInsetType: Int
        get() = if (Build.VERSION.SDK_INT >= 30) android.view.WindowInsets.Type.navigationBars() else 0
}
