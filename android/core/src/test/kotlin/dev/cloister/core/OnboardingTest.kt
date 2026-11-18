package dev.cloister.core

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** First run on the phone: philosophy, sign-in, each grant in turn, settings, optional Hardcore Mode. */
class OnboardingTest {
    private fun walk(o: Onboarding, p: PhoneProgress): List<OnboardingStep> {
        val seen = mutableListOf(o.step)
        while (true) seen += o.next(p) ?: break
        return seen
    }

    @Test
    fun aNewPhoneIsWalkedThroughEveryGrantOneAtATime() {
        assertEquals(
            listOf(
                OnboardingStep.Philosophy(0), OnboardingStep.Philosophy(1), OnboardingStep.Philosophy(2),
                OnboardingStep.SignIn, OnboardingStep.Vpn, OnboardingStep.AppBlocker, OnboardingStep.DeviceAdmin,
                OnboardingStep.Workdays, OnboardingStep.Hardcore,
            ),
            walk(Onboarding(), PhoneProgress()),
        )
    }

    @Test
    fun grantsAlreadyGivenAreSkipped() {
        val p = PhoneProgress(signedIn = true, vpnOn = true, appBlockerOn = true, adminActive = true, settingsSet = true)
        assertEquals(
            listOf(OnboardingStep.Philosophy(0), OnboardingStep.Philosophy(1), OnboardingStep.Philosophy(2), OnboardingStep.Hardcore),
            walk(Onboarding(), p),
        )
    }

    @Test
    fun hardcoreModeIsSkippedWhenAlreadyOn() {
        val p = PhoneProgress(signedIn = true, vpnOn = true, appBlockerOn = true, adminActive = true, settingsSet = true, deviceOwner = true)
        assertEquals(OnboardingStep.Philosophy(2), walk(Onboarding(), p).last())
    }

    @Test
    fun theSameSettingsAsTheDesktopWithTheSameChecks() {
        val form = WorkdaysForm(timeZone = "Europe/Amsterdam")
        assertEquals(Command.ChangeSettings(listOf(1, 2, 3, 4, 5), "09:00", "17:00", "Europe/Amsterdam"), form.command().getOrThrow())
        assertEquals("Mon to Fri, 09:00 to 17:00. Evening Lockdown starts at 17:00.", form.summary())
        val odd = form.toggle(5).toggle(6).copy(workdayStart = "8:30", shutdownTime = "16:15")
        assertEquals(Command.ChangeSettings(listOf(1, 2, 3, 4, 6), "08:30", "16:15", "Europe/Amsterdam"), odd.command().getOrThrow())
        assertEquals("Times are written like 09:00 or 17:30.", form.copy(shutdownTime = "25:00").command().exceptionOrNull()?.message)
        assertEquals("Shutdown Time comes after Workday Start.", form.copy(shutdownTime = "08:00").command().exceptionOrNull()?.message)
        assertEquals("Choose at least one Workday.", form.copy(workdays = emptySet()).command().exceptionOrNull()?.message)
    }

    @Test
    fun theHardcoreGuideEndsOnlyWhenTheAppIsDeviceOwner() {
        assertEquals("adb shell dpm set-device-owner dev.cloister/.app.enforce.AdminReceiver", HardcoreGuide.ADB_COMMAND)
        var g = HardcoreGuide()
        assertEquals(HardcoreStep.RemoveAccounts, g.step)
        g = g.next(deviceOwner = false)
        assertEquals(HardcoreStep.RunAdb, g.step)
        g = g.next(deviceOwner = false)
        assertEquals(HardcoreStep.Verify, g.step)
        val notYet = g.next(deviceOwner = false)
        assertEquals(HardcoreStep.Verify, notYet.step)
        assertEquals("Not yet. Check that every account is removed and run the command again.", notYet.notice)
        val done = g.next(deviceOwner = true)
        assertEquals(HardcoreStep.AddAccountsBack, done.step)
        assertNull(done.notice)
        assertTrue(done.finished)
        assertFalse(g.finished)
    }
}
