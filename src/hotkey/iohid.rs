//! IOKit HID bindings for FN/Globe key capture on macOS
//!
//! The FN key on Apple keyboards is handled at the firmware level and doesn't
//! generate normal CGEvent key events. To capture it, we need to use the lower-level
//! IOKit HID API to directly monitor keyboard input values.
//!
//! This module provides:
//! - Detection of Apple keyboards (internal and Magic Keyboard)
//! - FN key press/release monitoring via IOHIDManager
//! - Integration with the hotkey listener system

use core_foundation::base::{kCFAllocatorDefault, CFRelease, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRef};
use core_foundation::string::CFString;
use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::HotkeyEvent;

// Wrapper to make IOHIDManagerRef Send + Sync
// This is safe because:
// 1. IOHIDManager operations are atomic/thread-safe at the IOKit level
// 2. We only access the manager from a single context (start/stop)
// 3. The callback runs on the RunLoop thread but doesn't access the manager directly
struct SendSyncManager(IOHIDManagerRef);
unsafe impl Send for SendSyncManager {}
unsafe impl Sync for SendSyncManager {}

// IOKit HID constants
const K_IOHID_VENDOR_ID_KEY: &str = "VendorID";
const K_IOHID_PRODUCT_ID_KEY: &str = "ProductID";
const K_IOHID_DEVICE_USAGE_PAGE_KEY: &str = "DeviceUsagePage";
const K_IOHID_DEVICE_USAGE_KEY: &str = "DeviceUsage";

// HID Usage Pages
const K_HID_PAGE_GENERIC_DESKTOP: i32 = 0x01;
const K_HID_PAGE_APPLE_VENDOR_TOP_CASE: i32 = 0xFF;

// HID Usages
const K_HID_USAGE_GD_KEYBOARD: i32 = 0x06;

// Apple Vendor ID
const APPLE_VENDOR_ID: i32 = 0x05AC;

// Apple FN key usage (in Apple vendor-specific top case page)
const APPLE_FN_KEY_USAGE: i32 = 0x03;

// Known Apple keyboard product IDs (internal keyboards)
const APPLE_INTERNAL_KEYBOARD_IDS: &[i32] = &[
    0x0259, // MacBook (2015)
    0x0262, // MacBook Pro (2016)
    0x0263, // MacBook Pro (2016)
    0x0267, // MacBook Pro (2017)
    0x0272, // MacBook Air (2018)
    0x0273, // MacBook Pro (2018)
    0x027A, // MacBook Air (2019)
    0x027B, // MacBook Pro (2019)
    0x027C, // MacBook Air (2020)
    0x027D, // MacBook Pro (2020)
    0x027E, // MacBook Air (M1)
    0x027F, // MacBook Pro (M1)
    0x0340, // MacBook Air (M2)
    0x0341, // MacBook Pro (M2)
    0x0342, // MacBook Pro (M2 Pro/Max)
    0x0343, // MacBook Air (M3)
    0x0344, // MacBook Pro (M3)
];

// Known Apple Magic Keyboard product IDs
const APPLE_MAGIC_KEYBOARD_IDS: &[i32] = &[
    0x022C, // Magic Keyboard (2015)
    0x0267, // Magic Keyboard 2
    0x029C, // Magic Keyboard with Touch ID
    0x029D, // Magic Keyboard with Touch ID and Numeric Keypad
];

// FFI declarations for IOKit HID
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(
        allocator: *const c_void,
        options: u32,
    ) -> IOHIDManagerRef;

    fn IOHIDManagerSetDeviceMatching(
        manager: IOHIDManagerRef,
        matching: CFDictionaryRef,
    );

    fn IOHIDManagerRegisterInputValueCallback(
        manager: IOHIDManagerRef,
        callback: IOHIDValueCallback,
        context: *mut c_void,
    );

    fn IOHIDManagerScheduleWithRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: *const c_void,
    );

    fn IOHIDManagerUnscheduleFromRunLoop(
        manager: IOHIDManagerRef,
        run_loop: CFRunLoopRef,
        run_loop_mode: *const c_void,
    );

    fn IOHIDManagerOpen(
        manager: IOHIDManagerRef,
        options: u32,
    ) -> i32;

    fn IOHIDManagerClose(
        manager: IOHIDManagerRef,
        options: u32,
    ) -> i32;

    fn IOHIDManagerCopyDevices(
        manager: IOHIDManagerRef,
    ) -> *const c_void; // CFSetRef

    fn CFSetGetCount(
        theSet: *const c_void,
    ) -> isize;

    fn CFSetGetValues(
        theSet: *const c_void,
        values: *mut *const c_void,
    );

    fn IOHIDDeviceGetProperty(
        device: IOHIDDeviceRef,
        key: *const c_void, // CFStringRef
    ) -> *const c_void; // CFTypeRef

    fn IOHIDValueGetElement(
        value: IOHIDValueRef,
    ) -> IOHIDElementRef;

    fn IOHIDValueGetIntegerValue(
        value: IOHIDValueRef,
    ) -> i64;

    fn IOHIDElementGetUsagePage(
        element: IOHIDElementRef,
    ) -> u32;

    fn IOHIDElementGetUsage(
        element: IOHIDElementRef,
    ) -> u32;
}

// Opaque types
#[repr(C)]
pub struct __IOHIDManager {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}
pub type IOHIDManagerRef = *mut __IOHIDManager;

#[repr(C)]
pub struct __IOHIDDevice {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}
pub type IOHIDDeviceRef = *mut __IOHIDDevice;

#[repr(C)]
pub struct __IOHIDValue {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}
pub type IOHIDValueRef = *mut __IOHIDValue;

#[repr(C)]
pub struct __IOHIDElement {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}
pub type IOHIDElementRef = *mut __IOHIDElement;

// Callback type
type IOHIDValueCallback = extern "C" fn(
    context: *mut c_void,
    result: i32,
    sender: *mut c_void,
    value: IOHIDValueRef,
);

/// Minimum hold duration before triggering recording (milliseconds)
/// This allows quick taps to pass through for language switching
const FN_HOLD_THRESHOLD_MS: u64 = 200;

/// Raw events from the HID callback (before timing processing)
#[derive(Debug, Clone, Copy)]
enum RawFnEvent {
    Pressed,
    Released,
}

/// Context passed to the HID callback
struct HIDCallbackContext {
    sender: std::sync::mpsc::Sender<RawFnEvent>,
    fn_key_pressed: AtomicBool,
}

/// Check if an Apple keyboard with FN key support is available
pub fn has_apple_fn_keyboard() -> bool {
    unsafe {
        let manager = IOHIDManagerCreate(kCFAllocatorDefault, 0);
        if manager.is_null() {
            return false;
        }

        // Match keyboards
        let matching = create_keyboard_matching_dict();
        if let Some(dict) = matching {
            IOHIDManagerSetDeviceMatching(manager, dict.as_concrete_TypeRef());
        } else {
            IOHIDManagerSetDeviceMatching(manager, ptr::null());
        }

        // Open manager
        let result = IOHIDManagerOpen(manager, 0);
        if result != 0 {
            CFRelease(manager as CFTypeRef);
            return false;
        }

        // Get devices
        let devices_set = IOHIDManagerCopyDevices(manager);
        if devices_set.is_null() {
            IOHIDManagerClose(manager, 0);
            CFRelease(manager as CFTypeRef);
            return false;
        }

        let has_fn = check_for_apple_fn_keyboard(devices_set);

        CFRelease(devices_set);
        IOHIDManagerClose(manager, 0);
        CFRelease(manager as CFTypeRef);

        has_fn
    }
}

/// Check if any device in the set is an Apple keyboard with FN support
unsafe fn check_for_apple_fn_keyboard(devices_set: *const c_void) -> bool {
    let count = CFSetGetCount(devices_set);
    if count <= 0 {
        tracing::debug!("No HID devices found");
        return false;
    }

    // Allocate buffer for device pointers
    let mut devices: Vec<*const c_void> = vec![ptr::null(); count as usize];
    CFSetGetValues(devices_set, devices.as_mut_ptr());

    for device_ptr in devices {
        if device_ptr.is_null() {
            continue;
        }
        let device = device_ptr as IOHIDDeviceRef;
        if is_apple_fn_keyboard(device) {
            tracing::debug!("Found Apple keyboard with FN key support");
            return true;
        }
    }

    tracing::debug!("No Apple keyboard with FN key support found");
    false
}

/// Check if a device is an Apple keyboard that supports FN key
unsafe fn is_apple_fn_keyboard(device: IOHIDDeviceRef) -> bool {
    // Get vendor ID
    let vendor_key = CFString::new(K_IOHID_VENDOR_ID_KEY);
    let vendor_prop = IOHIDDeviceGetProperty(device, vendor_key.as_concrete_TypeRef() as *const _);

    if vendor_prop.is_null() {
        return false;
    }

    let vendor_num: CFNumber = CFNumber::wrap_under_get_rule(vendor_prop as *const _);
    let vendor_id: i32 = vendor_num.to_i32().unwrap_or(0);

    if vendor_id != APPLE_VENDOR_ID {
        return false;
    }

    // Get product ID
    let product_key = CFString::new(K_IOHID_PRODUCT_ID_KEY);
    let product_prop = IOHIDDeviceGetProperty(device, product_key.as_concrete_TypeRef() as *const _);

    if product_prop.is_null() {
        // Unknown Apple keyboard - assume it has FN
        tracing::debug!("Apple keyboard with unknown product ID, assuming FN support");
        return true;
    }

    let product_num: CFNumber = CFNumber::wrap_under_get_rule(product_prop as *const _);
    let product_id: i32 = product_num.to_i32().unwrap_or(0);

    // Check against known keyboards
    let is_internal = APPLE_INTERNAL_KEYBOARD_IDS.contains(&product_id);
    let is_magic = APPLE_MAGIC_KEYBOARD_IDS.contains(&product_id);

    if is_internal {
        tracing::debug!("Found Apple internal keyboard (product ID: 0x{:04X})", product_id);
        return true;
    }

    if is_magic {
        tracing::debug!("Found Apple Magic Keyboard (product ID: 0x{:04X})", product_id);
        return true;
    }

    // Unknown Apple keyboard - assume it has FN for internal keyboards
    // (any Apple keyboard should have FN)
    tracing::debug!("Apple keyboard (product ID: 0x{:04X}), assuming FN support", product_id);
    true
}

/// Create matching dictionary for keyboards
fn create_keyboard_matching_dict() -> Option<CFDictionary<CFString, CFNumber>> {
    let usage_page = CFNumber::from(K_HID_PAGE_GENERIC_DESKTOP);
    let usage = CFNumber::from(K_HID_USAGE_GD_KEYBOARD);

    let keys = vec![
        CFString::new(K_IOHID_DEVICE_USAGE_PAGE_KEY),
        CFString::new(K_IOHID_DEVICE_USAGE_KEY),
    ];
    let values = vec![usage_page, usage];

    Some(CFDictionary::from_CFType_pairs(&keys.iter().zip(values.iter()).map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>()))
}

/// FN key listener using IOHIDManager
pub struct FnKeyListener {
    manager: Option<SendSyncManager>,
    callback_context: Option<Box<HIDCallbackContext>>,
    stop_flag: Arc<AtomicBool>,
}

impl FnKeyListener {
    /// Create a new FN key listener
    pub fn new() -> Self {
        Self {
            manager: None,
            callback_context: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start listening for FN key events
    /// Returns a channel receiver for events
    pub fn start(&mut self) -> Result<mpsc::Receiver<HotkeyEvent>, crate::error::HotkeyError> {
        let (async_tx, async_rx) = mpsc::channel(32);

        // Create callback context with raw event sender
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<RawFnEvent>();
        let context = Box::new(HIDCallbackContext {
            sender: raw_tx,
            fn_key_pressed: AtomicBool::new(false),
        });
        let context_ptr = Box::into_raw(context);

        unsafe {
            // Create HID manager
            let manager = IOHIDManagerCreate(kCFAllocatorDefault, 0);
            if manager.is_null() {
                let _ = Box::from_raw(context_ptr);
                return Err(crate::error::HotkeyError::DeviceAccess(
                    "Failed to create IOHIDManager".to_string(),
                ));
            }

            // Match keyboards with Apple vendor-specific top case usage
            // This captures the FN key events
            IOHIDManagerSetDeviceMatching(manager, ptr::null());

            // Register callback
            IOHIDManagerRegisterInputValueCallback(
                manager,
                hid_value_callback,
                context_ptr as *mut c_void,
            );

            // Schedule with run loop
            IOHIDManagerScheduleWithRunLoop(
                manager,
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopDefaultMode as *const c_void,
            );

            // Open manager
            let result = IOHIDManagerOpen(manager, 0);
            if result != 0 {
                IOHIDManagerUnscheduleFromRunLoop(
                    manager,
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    kCFRunLoopDefaultMode as *const c_void,
                );
                CFRelease(manager as CFTypeRef);
                let _ = Box::from_raw(context_ptr);
                return Err(crate::error::HotkeyError::DeviceAccess(
                    format!("Failed to open IOHIDManager: error {}", result),
                ));
            }

            self.manager = Some(SendSyncManager(manager));
            self.callback_context = Some(Box::from_raw(context_ptr));

            tracing::info!("FN key listener started (IOHIDManager)");
        }

        // Spawn task to process raw events with tap-vs-hold timing
        // Strategy: Start recording INSTANTLY on press (zero latency), then:
        // - If released quickly (tap): Cancel recording, let language switch through
        // - If held long enough: Complete recording normally
        let stop_flag = self.stop_flag.clone();
        let async_tx_clone = async_tx.clone();

        std::thread::spawn(move || {
            let mut press_time: Option<std::time::Instant> = None;

            while !stop_flag.load(Ordering::SeqCst) {
                match raw_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                    Ok(RawFnEvent::Pressed) => {
                        press_time = Some(std::time::Instant::now());
                        // Start recording IMMEDIATELY - zero latency
                        let _ = async_tx_clone.blocking_send(HotkeyEvent::Pressed);
                        tracing::debug!("FN key pressed, recording started instantly");
                    }
                    Ok(RawFnEvent::Released) => {
                        if let Some(t) = press_time {
                            let held_ms = t.elapsed().as_millis();
                            if held_ms >= FN_HOLD_THRESHOLD_MS as u128 {
                                // Held long enough - complete the recording
                                let _ = async_tx_clone.blocking_send(HotkeyEvent::Released);
                                tracing::debug!("FN key released after {}ms (recording completed)", held_ms);
                            } else {
                                // Quick tap - cancel recording, let language switch through
                                let _ = async_tx_clone.blocking_send(HotkeyEvent::Cancel);
                                tracing::debug!("FN key tapped ({}ms < {}ms threshold, cancelled for language switch)",
                                    held_ms, FN_HOLD_THRESHOLD_MS);
                            }
                        }
                        press_time = None;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        Ok(async_rx)
    }

    /// Stop listening
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);

        if let Some(SendSyncManager(manager)) = self.manager.take() {
            unsafe {
                IOHIDManagerUnscheduleFromRunLoop(
                    manager,
                    CFRunLoop::get_current().as_concrete_TypeRef(),
                    kCFRunLoopDefaultMode as *const c_void,
                );
                IOHIDManagerClose(manager, 0);
                CFRelease(manager as CFTypeRef);
            }
        }

        self.callback_context = None;
        tracing::debug!("FN key listener stopped");
    }
}

impl Drop for FnKeyListener {
    fn drop(&mut self) {
        self.stop();
    }
}

/// HID value callback - called when any HID value changes
/// Sends raw press/release events to be processed by the timing logic
extern "C" fn hid_value_callback(
    context: *mut c_void,
    _result: i32,
    _sender: *mut c_void,
    value: IOHIDValueRef,
) {
    if context.is_null() || value.is_null() {
        return;
    }

    unsafe {
        let ctx = &*(context as *const HIDCallbackContext);

        let element = IOHIDValueGetElement(value);
        if element.is_null() {
            return;
        }

        let usage_page = IOHIDElementGetUsagePage(element);
        let usage = IOHIDElementGetUsage(element);
        let int_value = IOHIDValueGetIntegerValue(value);

        // Check for FN key (Apple vendor top case page, usage 0x03)
        if usage_page == K_HID_PAGE_APPLE_VENDOR_TOP_CASE as u32 && usage == APPLE_FN_KEY_USAGE as u32 {
            let was_pressed = ctx.fn_key_pressed.load(Ordering::SeqCst);
            let is_pressed = int_value != 0;

            if is_pressed && !was_pressed {
                ctx.fn_key_pressed.store(true, Ordering::SeqCst);
                let _ = ctx.sender.send(RawFnEvent::Pressed);
            } else if !is_pressed && was_pressed {
                ctx.fn_key_pressed.store(false, Ordering::SeqCst);
                let _ = ctx.sender.send(RawFnEvent::Released);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_apple_fn_keyboard() {
        // This test will pass on Macs with Apple keyboards
        let has_fn = has_apple_fn_keyboard();
        println!("Has Apple FN keyboard: {}", has_fn);
    }

    #[test]
    fn test_known_keyboard_ids() {
        // Verify we have reasonable keyboard ID lists
        assert!(!APPLE_INTERNAL_KEYBOARD_IDS.is_empty());
        assert!(!APPLE_MAGIC_KEYBOARD_IDS.is_empty());
    }
}
