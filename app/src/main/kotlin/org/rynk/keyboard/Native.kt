// SPDX-License-Identifier: GPL-3.0-or-later
package org.rynk.keyboard

import android.view.Surface

/** Bridge to the Rust core (librynk_core.so). All calls except [kbModel] happen on the main thread. */
object Native {
    init {
        System.loadLibrary("rynk_core")
    }

    // keyboard
    external fun kbInit(filesDir: String, locales: String, night: Boolean)
    external fun kbNight(night: Boolean)
    external fun kbMeasure(width: Int, density: Float, bottomInset: Int): Int
    external fun kbAttach(surface: Surface): Boolean
    external fun kbResize(width: Int, height: Int)
    external fun kbDetach()
    external fun kbStartInput(inputType: Int, imeOptions: Int, restarting: Boolean)
    external fun kbFinishInput()
    external fun kbContext(before: String, caps: Boolean)
    external fun kbAbandon()
    external fun kbTouch(action: Int, id: Int, x: Float, y: Float, time: Long)
    external fun kbTick(time: Long)
    external fun kbWake(): Long
    external fun kbRender()
    external fun kbNeedsRender(): Boolean
    external fun kbTakeCommands(): String
    external fun kbModel(code: String, data: ByteArray)

    // settings screen
    external fun stInit(filesDir: String, locales: String, night: Boolean)
    external fun stAttach(surface: Surface): Boolean
    external fun stResize(width: Int, height: Int, density: Float, top: Int, bottom: Int)
    external fun stDetach()
    external fun stImeState(enabled: Boolean, selected: Boolean, night: Boolean)
    external fun stTouch(action: Int, id: Int, x: Float, y: Float, time: Long)
    external fun stRender(): Boolean
    external fun stTakeCommands(): String
    external fun stBack(): Boolean
}
