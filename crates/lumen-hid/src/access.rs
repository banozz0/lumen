//! Whether macOS will let this process open a HID device.
//!
//! The grant is **Input Monitoring**, and macOS charges it to the application
//! that launched the shell rather than to the `lumen` binary. Two things about
//! it are worth knowing before reading the code:
//!
//! * Enumerating devices works without the grant; opening one does not. That
//!   asymmetry is why a missing grant reads as broken code -- `lumen list`
//!   prints a healthy device table while every write fails.
//! * A refusal is permanent. Once the answer is `Denied` macOS never prompts
//!   again, so a tool that waits for a prompt waits forever. Only the System
//!   Settings toggle (or `tccutil reset ListenEvent <bundle-id>`) undoes it.
//!
//! Asking `IOHIDCheckAccess` is the only way to tell those apart: the failure
//! that reaches us from an open is `(iokit/common) not permitted` either way.

/// What macOS says about this process's Input Monitoring access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Granted: opening devices works.
    Granted,
    /// Refused at some point. macOS will not ask again.
    Denied,
    /// Never asked. Opening a device makes macOS put up the prompt.
    Unknown,
}

impl Access {
    /// What the user has to do about it, phrased for the end of an error line.
    /// `Granted` has no fix, so it has no text.
    pub fn fix(self) -> Option<&'static str> {
        match self {
            Access::Granted => None,
            Access::Denied => Some(
                "your terminal is already in that list with its switch off. Turn it on \
                 -- macOS will not ask again by itself -- then run this again.",
            ),
            Access::Unknown => Some(
                "add your terminal to that list, or run this again and allow the prompt \
                 macOS puts up.",
            ),
        }
    }
}

/// `IOHIDRequestType`: listening to HID events, which is what Input Monitoring
/// governs. `kIOHIDRequestTypePostEvent` is 0 and is a different grant.
const REQUEST_LISTEN_EVENT: i32 = 1;

// IOKit/hidsystem/IOHIDLib.h, macOS 10.15+. `IOHIDAccessType` is a plain C enum:
// granted 0, denied 1, unknown 2.
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOHIDCheckAccess(request_type: i32) -> i32;
}

/// Ask macOS whether this process may open HID devices. A pure query: it never
/// prompts, so it is safe to call before doing anything.
pub fn input_monitoring() -> Access {
    match unsafe { IOHIDCheckAccess(REQUEST_LISTEN_EVENT) } {
        0 => Access::Granted,
        1 => Access::Denied,
        _ => Access::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The call must not crash or hang, whatever this machine's grant is. Which
    /// of the three it returns is a property of the machine, not of the code, so
    /// there is nothing else to assert here.
    #[test]
    fn checking_access_returns_one_of_the_three_answers() {
        assert!(matches!(
            input_monitoring(),
            Access::Granted | Access::Denied | Access::Unknown
        ));
    }

    /// Every state a user can act on has to say what to do about it.
    #[test]
    fn only_granted_has_nothing_to_fix() {
        assert!(Access::Granted.fix().is_none());
        assert!(Access::Denied.fix().unwrap().contains("Turn it on"));
        assert!(Access::Unknown.fix().unwrap().contains("prompt"));
    }
}
