package dev.cloister.core

/**
 * First run on the phone (same model as the desktop's `cloister_app_model::Onboarding`): a short
 * philosophy, sign-in, the VPN, the app blocker (Accessibility) and Device Admin one at a time,
 * the same Workdays settings as the desktop, then the optional Hardcore Mode guide. Steps
 * already done are skipped, so onboarding resumes where the user left (e.g. after a system screen).
 */
sealed interface OnboardingStep {
    data class Philosophy(val page: Int) : OnboardingStep
    data object SignIn : OnboardingStep
    data object Vpn : OnboardingStep
    data object AppBlocker : OnboardingStep
    data object DeviceAdmin : OnboardingStep
    data object Workdays : OnboardingStep
    /** Optional, and the last step. */
    data object Hardcore : OnboardingStep
}

/** What is already set up on this phone. */
data class PhoneProgress(
    val signedIn: Boolean = false,
    val vpnOn: Boolean = false,
    val appBlockerOn: Boolean = false,
    val adminActive: Boolean = false,
    val settingsSet: Boolean = false,
    val deviceOwner: Boolean = false,
)

data class Page(val title: String, val body: String)

class Onboarding(start: OnboardingStep = OnboardingStep.Philosophy(0)) {
    var step: OnboardingStep = start
        private set
    private val history = ArrayDeque<OnboardingStep>()

    /** Move to the next step not yet done; null once past the last one. */
    fun next(p: PhoneProgress): OnboardingStep? {
        var candidate = step
        while (true) {
            candidate = when (candidate) {
                is OnboardingStep.Philosophy ->
                    if (candidate.page + 1 < PHILOSOPHY.size) OnboardingStep.Philosophy(candidate.page + 1) else OnboardingStep.SignIn
                OnboardingStep.SignIn -> OnboardingStep.Vpn
                OnboardingStep.Vpn -> OnboardingStep.AppBlocker
                OnboardingStep.AppBlocker -> OnboardingStep.DeviceAdmin
                OnboardingStep.DeviceAdmin -> OnboardingStep.Workdays
                OnboardingStep.Workdays -> OnboardingStep.Hardcore
                OnboardingStep.Hardcore -> return null
            }
            val done = when (candidate) {
                OnboardingStep.SignIn -> p.signedIn
                OnboardingStep.Vpn -> p.vpnOn
                OnboardingStep.AppBlocker -> p.appBlockerOn
                OnboardingStep.DeviceAdmin -> p.adminActive
                OnboardingStep.Workdays -> p.settingsSet
                OnboardingStep.Hardcore -> p.deviceOwner
                is OnboardingStep.Philosophy -> false
            }
            if (!done) {
                history.addLast(step)
                step = candidate
                return candidate
            }
        }
    }

    fun back(): OnboardingStep {
        history.removeLastOrNull()?.let { step = it }
        return step
    }

    companion object {
        /** The same three screens as the desktop. */
        val PHILOSOPHY = listOf(
            Page(
                "Deep work is rare, and valuable.",
                "Focus without distraction on a hard problem is the work that moves things. It can't be done in the gaps between notifications.",
            ),
            Page(
                "Willpower isn't the plan. Rules are.",
                "A Deep Session can't be cancelled, on any Device, until it ends. Rules tighten at once and loosen only tomorrow, so a weak moment can't undo a good decision.",
            ),
            Page(
                "Work ends, on purpose.",
                "At Shutdown Time you capture Loose Ends and plan tomorrow. Then work tools stay closed until morning. Rest is part of the method.",
            ),
        )
    }
}

/** Workdays, Workday Start and Shutdown Time, as on the desktop. */
data class WorkdaysForm(
    val workdays: Set<Int> = (1..5).toSet(),
    val workdayStart: String = "09:00",
    val shutdownTime: String = "17:00",
    val timeZone: String,
) {
    fun toggle(day: Int) = copy(workdays = if (day in workdays) workdays - day else workdays + day)

    fun command(): Result<Command.ChangeSettings> {
        val times = "Times are written like 09:00 or 17:30."
        val start = parse(workdayStart) ?: return Result.failure(IllegalArgumentException(times))
        val end = parse(shutdownTime) ?: return Result.failure(IllegalArgumentException(times))
        if (workdays.isEmpty()) return Result.failure(IllegalArgumentException("Choose at least one Workday."))
        if (end <= start) return Result.failure(IllegalArgumentException("Shutdown Time comes after Workday Start."))
        return Result.success(Command.ChangeSettings(workdays.sorted(), hhmm(start), hhmm(end), timeZone))
    }

    fun summary(): String {
        val days = workdays.sorted()
        val consecutive = days.zipWithNext().all { (a, b) -> b == a + 1 }
        val dayText = if (days.size >= 3 && consecutive) "${dayName(days.first())} to ${dayName(days.last())}"
        else days.joinToString(", ") { dayName(it) }
        val s = parse(workdayStart) ?: return dayText
        val e = parse(shutdownTime) ?: return dayText
        return "$dayText, ${hhmm(s)} to ${hhmm(e)}. Evening Lockdown starts at ${hhmm(e)}."
    }

    companion object {
        private val NAMES = listOf("Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun")
        fun dayName(day: Int) = NAMES[(day.coerceIn(1, 7)) - 1]

        private fun parse(s: String): Int? {
            val t = s.trim()
            if (t.length > 5) return null
            val parts = t.split(":")
            if (parts.size != 2 || parts[1].length != 2) return null
            val h = parts[0].toIntOrNull() ?: return null
            val m = parts[1].toIntOrNull() ?: return null
            return if (h in 0..23 && m in 0..59) h * 60 + m else null
        }

        private fun hhmm(minutes: Int) = "%02d:%02d".format(minutes / 60, minutes % 60)
    }
}

enum class HardcoreStep { RemoveAccounts, RunAdb, Verify, AddAccountsBack }

/**
 * Hardcore Mode: making Cloister the Device Owner, which Android only allows on a phone with no
 * accounts. Remove the accounts for a moment, run one adb command from a computer, verify, then
 * add the accounts back. Once active the VPN and the app can't be turned off.
 */
data class HardcoreGuide(val step: HardcoreStep = HardcoreStep.RemoveAccounts, val notice: String? = null) {
    val finished get() = step == HardcoreStep.AddAccountsBack

    fun next(deviceOwner: Boolean): HardcoreGuide = when (step) {
        HardcoreStep.RemoveAccounts -> HardcoreGuide(HardcoreStep.RunAdb)
        HardcoreStep.RunAdb -> HardcoreGuide(HardcoreStep.Verify)
        HardcoreStep.Verify ->
            if (deviceOwner) HardcoreGuide(HardcoreStep.AddAccountsBack)
            else copy(notice = "Not yet. Check that every account is removed and run the command again.")
        HardcoreStep.AddAccountsBack -> this
    }

    companion object {
        const val ADB_COMMAND = "adb shell dpm set-device-owner dev.cloister/.app.enforce.AdminReceiver"
    }
}
