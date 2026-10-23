//! The Vita's system keyboard (the IME common dialog), where retail opened the Xbox one (`XamShowKeyboardUI`, FE 46,
//! research b305). One dialog at a time: [`Keyboard::open`] starts it, [`Keyboard::poll`] once a frame gives the text
//! (Enter) or nothing (Close) when it finishes. While it runs the frame swap must let the dialog draw
//! ([`Keyboard::running`]: `vglSwapBuffers(GL_TRUE)`).
use vitasdk_sys::*;

/// `PSP2_SDK_VERSION` and `SCE_COMMON_DIALOG_MAGIC_NUMBER` (psp2common/defs.h, psp2/common_dialog.h).
const SDK_VERSION: u32 = 0x0357_0011;
const MAGIC: u32 = 0xC0D1_A109;
/// The longest text the dialog takes (UTF-16 units).
const MAX_LEN: usize = 128;

#[derive(Default)]
pub struct Keyboard {
    /// The dialog's buffers, alive (and not moved: boxed) while it runs.
    buffers: Option<Box<Buffers>>,
}

struct Buffers {
    title: [u16; 64],
    input: [u16; MAX_LEN + 1],
    param: SceImeDialogParam,
}

impl Keyboard {
    /// Opens the keyboard titled `title`, taking at most `max_len` characters (capped at 128). False when the system
    /// refused it (another dialog runs).
    pub fn open(&mut self, title: &str, max_len: usize) -> bool {
        if self.buffers.is_some() {
            return false;
        }
        let mut b = Box::new(Buffers { title: [0; 64], input: [0; MAX_LEN + 1], param: unsafe { core::mem::zeroed() } });
        for (dst, src) in b.title.iter_mut().zip(title.encode_utf16().take(63)) {
            *dst = src;
        }
        let input = b.input.as_mut_ptr();
        let title = b.title.as_ptr();
        let p = &mut b.param;
        // sceImeDialogParamInit (inline in ime_dialog.h): zeroed, the magic plus the common parameter's address.
        p.commonParam.magic = MAGIC.wrapping_add(&p.commonParam as *const _ as u32);
        p.sdkVersion = SDK_VERSION;
        p.supportedLanguages = 0;
        p.languagesForced = 0;
        p.type_ = SCE_IME_TYPE_BASIC_LATIN;
        p.option = SCE_IME_OPTION_NO_AUTO_CAPITALIZATION;
        p.dialogMode = SCE_IME_DIALOG_DIALOG_MODE_DEFAULT;
        p.textBoxMode = SCE_IME_DIALOG_TEXTBOX_MODE_DEFAULT;
        p.title = title;
        p.maxTextLength = max_len.clamp(1, MAX_LEN) as u32;
        p.initialText = input;
        p.inputTextBuffer = input;
        if unsafe { sceImeDialogInit(&b.param) } < 0 {
            return false;
        }
        self.buffers = Some(b);
        true
    }

    /// The dialog is up (the swap must draw it).
    pub fn running(&self) -> bool {
        self.buffers.is_some()
    }

    /// Once a frame while it runs: Some(Some(text)) when entered, Some(None) when closed, None while it runs.
    pub fn poll(&mut self) -> Option<Option<String>> {
        let b = self.buffers.as_ref()?;
        if unsafe { sceImeDialogGetStatus() } != SCE_COMMON_DIALOG_STATUS_FINISHED {
            return None;
        }
        let mut result: SceImeDialogResult = unsafe { core::mem::zeroed() };
        unsafe { sceImeDialogGetResult(&mut result) };
        let len = b.input.iter().position(|&c| c == 0).unwrap_or(b.input.len());
        let text = (result.button == SCE_IME_DIALOG_BUTTON_ENTER as i32).then(|| String::from_utf16_lossy(&b.input[..len]));
        unsafe { sceImeDialogTerm() };
        self.buffers = None;
        Some(text)
    }
}
