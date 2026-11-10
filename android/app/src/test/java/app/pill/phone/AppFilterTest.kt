package app.pill.phone

import app.pill.phone.data.AppFilter
import app.pill.phone.service.NotifBridge
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AppFilterTest {
    @Test fun defaults() {
        assertTrue(AppFilter.isEnabled("org.telegram.messenger", false, emptyMap()))
        assertTrue(AppFilter.isEnabled("com.google.android.gm", true, emptyMap()))   // known app wins over "system"
        assertTrue(AppFilter.isEnabled("some.random.app", false, emptyMap()))
        assertFalse(AppFilter.isEnabled("com.android.systemui", true, emptyMap()))
        assertFalse(AppFilter.isEnabled(AppFilter.SELF, false, mapOf(AppFilter.SELF to true)))
    }

    @Test fun userChoiceWins() {
        assertFalse(AppFilter.isEnabled("org.telegram.messenger", false, mapOf("org.telegram.messenger" to false)))
        assertTrue(AppFilter.isEnabled("com.android.systemui", true, mapOf("com.android.systemui" to true)))
    }

    @Test fun categories() {
        assertEquals("msg", NotifBridge.category("msg"))
        assertEquals("call", NotifBridge.category("call"))
        assertEquals("email", NotifBridge.category("email"))
        assertEquals("other", NotifBridge.category(null))
        assertEquals("other", NotifBridge.category("whatever"))
    }
}
