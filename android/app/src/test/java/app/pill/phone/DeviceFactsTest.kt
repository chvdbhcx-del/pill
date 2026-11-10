package app.pill.phone

import android.media.AudioManager
import android.os.BatteryManager
import android.telephony.TelephonyManager
import app.pill.phone.service.DeviceFacts
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DeviceFactsTest {
    @Test fun chargingStateAndPlug() {
        assertEquals("charging", DeviceFacts.chargeOf(BatteryManager.BATTERY_STATUS_CHARGING))
        assertEquals("full", DeviceFacts.chargeOf(BatteryManager.BATTERY_STATUS_FULL))
        assertEquals("none", DeviceFacts.chargeOf(BatteryManager.BATTERY_STATUS_DISCHARGING))
        assertEquals("none", DeviceFacts.chargeOf(BatteryManager.BATTERY_STATUS_NOT_CHARGING))
        assertEquals("ac", DeviceFacts.plugOf(BatteryManager.BATTERY_PLUGGED_AC))
        assertEquals("ac", DeviceFacts.plugOf(8))
        assertEquals("usb", DeviceFacts.plugOf(BatteryManager.BATTERY_PLUGGED_USB))
        assertEquals("wireless", DeviceFacts.plugOf(BatteryManager.BATTERY_PLUGGED_WIRELESS))
        assertNull(DeviceFacts.plugOf(0))
    }

    @Test fun healthAndTemperature() {
        assertEquals("good", DeviceFacts.healthOf(BatteryManager.BATTERY_HEALTH_GOOD))
        assertEquals("overheat", DeviceFacts.healthOf(BatteryManager.BATTERY_HEALTH_OVERHEAT))
        assertEquals("over_voltage", DeviceFacts.healthOf(BatteryManager.BATTERY_HEALTH_OVER_VOLTAGE))
        assertEquals("unknown", DeviceFacts.healthOf(0))
        // tenths of a degree, rounded to half degrees so a flicker of 0.1 does not trigger a send
        assertEquals(31.5, DeviceFacts.tempC(315)!!, 0.0)
        assertEquals(31.5, DeviceFacts.tempC(313)!!, 0.0)
        assertEquals(32.0, DeviceFacts.tempC(318)!!, 0.0)
        assertNull(DeviceFacts.tempC(Int.MIN_VALUE))
        assertNull(DeviceFacts.tempC(5000))
    }

    @Test fun timeToFullIsOnlySentWhenItIsARealEstimate() {
        assertEquals(42, DeviceFacts.fullInMin(42 * 60_000L, true))
        assertEquals(1, DeviceFacts.fullInMin(10_000, true))
        assertNull(DeviceFacts.fullInMin(-1, true))
        assertNull(DeviceFacts.fullInMin(0, true))
        assertNull(DeviceFacts.fullInMin(42 * 60_000L, false))
        assertNull(DeviceFacts.fullInMin(30L * 24 * 3_600_000, true))
    }

    @Test fun storageAndMemoryAreRoundedAndNeverAboveTheTotal() {
        val gib = 1024L * 1024 * 1024
        val s = DeviceFacts.space(52 * gib + 100, 128 * gib)!!
        assertEquals(52 * gib, s.free)  // half-GiB steps
        assertEquals(128 * gib, s.total)
        assertEquals(128 * gib, DeviceFacts.space(129 * gib, 128 * gib)!!.free)
        assertNull(DeviceFacts.space(5, 0))
        assertNull(DeviceFacts.space(-1, 10))
        val mb = 1024L * 1024
        val r = DeviceFacts.ram(3100 * mb, 8000 * mb)!!
        assertEquals(3072L, r.free)  // 128 MB steps
        assertEquals(8000L, r.total)
        assertEquals(8000L, DeviceFacts.ram(9000 * mb, 8000 * mb)!!.free)
    }

    @Test fun ringerAndGeneration() {
        assertEquals("silent", DeviceFacts.ringerOf(AudioManager.RINGER_MODE_SILENT))
        assertEquals("vibrate", DeviceFacts.ringerOf(AudioManager.RINGER_MODE_VIBRATE))
        assertEquals("sound", DeviceFacts.ringerOf(AudioManager.RINGER_MODE_NORMAL))
        assertEquals("5G", DeviceFacts.genOf(TelephonyManager.NETWORK_TYPE_NR))
        assertEquals("4G", DeviceFacts.genOf(TelephonyManager.NETWORK_TYPE_LTE))
        assertEquals("3G", DeviceFacts.genOf(TelephonyManager.NETWORK_TYPE_HSPAP))
        assertEquals("2G", DeviceFacts.genOf(TelephonyManager.NETWORK_TYPE_EDGE))
        assertNull(DeviceFacts.genOf(TelephonyManager.NETWORK_TYPE_UNKNOWN))
    }
}
