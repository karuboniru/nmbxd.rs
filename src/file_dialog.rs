//! Native dialogs with a parent handle compatible with GPUI 0.2.2.

use gpui::Window;

pub fn for_window(window: &Window) -> rfd::AsyncFileDialog {
    #[cfg(target_os = "windows")]
    let parent = windows::DialogParent(window);
    #[cfg(not(target_os = "windows"))]
    let parent = window;

    rfd::AsyncFileDialog::new().set_parent(&parent)
}

#[cfg(target_os = "windows")]
mod windows {
    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
    };

    // RFD queries both handles, but GPUI 0.2.2's Windows display_handle()
    // panics. Keep the real HWND for ownership and modality, and supply the
    // stateless Windows display handle without calling GPUI's implementation.
    pub(super) struct DialogParent<'a>(pub(super) &'a gpui::Window);

    impl HasWindowHandle for DialogParent<'_> {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            HasWindowHandle::window_handle(self.0)
        }
    }

    impl HasDisplayHandle for DialogParent<'_> {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(DisplayHandle::windows())
        }
    }
}
