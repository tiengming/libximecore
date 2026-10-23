pub type KeyCode = u32;
pub type Modifier = u32;

pub const XK_BACK_SPACE: KeyCode = librime_sys2::RimeKeyCode_XK_BackSpace;
pub const XK_TAB: KeyCode = librime_sys2::RimeKeyCode_XK_Tab;
pub const XK_RETURN: KeyCode = librime_sys2::RimeKeyCode_XK_Return;
pub const XK_ESCAPE: KeyCode = librime_sys2::RimeKeyCode_XK_Escape;
pub const XK_DELETE: KeyCode = librime_sys2::RimeKeyCode_XK_Delete;
pub const XK_SPACE: KeyCode = librime_sys2::RimeKeyCode_XK_space;
pub const XK_LEFT: KeyCode = librime_sys2::RimeKeyCode_XK_Left;
pub const XK_UP: KeyCode = librime_sys2::RimeKeyCode_XK_Up;
pub const XK_RIGHT: KeyCode = librime_sys2::RimeKeyCode_XK_Right;
pub const XK_DOWN: KeyCode = librime_sys2::RimeKeyCode_XK_Down;
pub const XK_PRIOR: KeyCode = librime_sys2::RimeKeyCode_XK_Prior;
pub const XK_NEXT: KeyCode = librime_sys2::RimeKeyCode_XK_Next;
pub const XK_HOME: KeyCode = librime_sys2::RimeKeyCode_XK_Home;
pub const XK_END: KeyCode = librime_sys2::RimeKeyCode_XK_End;

pub const XK_SHIFT_L: KeyCode = librime_sys2::RimeKeyCode_XK_Shift_L;
pub const XK_SHIFT_R: KeyCode = librime_sys2::RimeKeyCode_XK_Shift_R;

pub const XK_A: KeyCode = librime_sys2::RimeKeyCode_XK_a;
pub const XK_B: KeyCode = librime_sys2::RimeKeyCode_XK_b;
pub const XK_C: KeyCode = librime_sys2::RimeKeyCode_XK_c;
pub const XK_D: KeyCode = librime_sys2::RimeKeyCode_XK_d;
pub const XK_E: KeyCode = librime_sys2::RimeKeyCode_XK_e;
pub const XK_F: KeyCode = librime_sys2::RimeKeyCode_XK_f;
pub const XK_G: KeyCode = librime_sys2::RimeKeyCode_XK_g;
pub const XK_H: KeyCode = librime_sys2::RimeKeyCode_XK_h;
pub const XK_I: KeyCode = librime_sys2::RimeKeyCode_XK_i;
pub const XK_J: KeyCode = librime_sys2::RimeKeyCode_XK_j;
pub const XK_K: KeyCode = librime_sys2::RimeKeyCode_XK_k;
pub const XK_L: KeyCode = librime_sys2::RimeKeyCode_XK_l;
pub const XK_M: KeyCode = librime_sys2::RimeKeyCode_XK_m;
pub const XK_N: KeyCode = librime_sys2::RimeKeyCode_XK_n;
pub const XK_O: KeyCode = librime_sys2::RimeKeyCode_XK_o;
pub const XK_P: KeyCode = librime_sys2::RimeKeyCode_XK_p;
pub const XK_Q: KeyCode = librime_sys2::RimeKeyCode_XK_q;
pub const XK_R: KeyCode = librime_sys2::RimeKeyCode_XK_r;
pub const XK_S: KeyCode = librime_sys2::RimeKeyCode_XK_s;
pub const XK_T: KeyCode = librime_sys2::RimeKeyCode_XK_t;
pub const XK_U: KeyCode = librime_sys2::RimeKeyCode_XK_u;
pub const XK_V: KeyCode = librime_sys2::RimeKeyCode_XK_v;
pub const XK_W: KeyCode = librime_sys2::RimeKeyCode_XK_w;
pub const XK_X: KeyCode = librime_sys2::RimeKeyCode_XK_x;
pub const XK_Y: KeyCode = librime_sys2::RimeKeyCode_XK_y;
pub const XK_Z: KeyCode = librime_sys2::RimeKeyCode_XK_z;

pub const XK_0: KeyCode = librime_sys2::RimeKeyCode_XK_0;
pub const XK_1: KeyCode = librime_sys2::RimeKeyCode_XK_1;
pub const XK_2: KeyCode = librime_sys2::RimeKeyCode_XK_2;
pub const XK_3: KeyCode = librime_sys2::RimeKeyCode_XK_3;
pub const XK_4: KeyCode = librime_sys2::RimeKeyCode_XK_4;
pub const XK_5: KeyCode = librime_sys2::RimeKeyCode_XK_5;
pub const XK_6: KeyCode = librime_sys2::RimeKeyCode_XK_6;
pub const XK_7: KeyCode = librime_sys2::RimeKeyCode_XK_7;
pub const XK_8: KeyCode = librime_sys2::RimeKeyCode_XK_8;
pub const XK_9: KeyCode = librime_sys2::RimeKeyCode_XK_9;

pub const K_SHIFT_MASK: Modifier = librime_sys2::RimeModifier_kShiftMask;
pub const K_LOCK_MASK: Modifier = librime_sys2::RimeModifier_kLockMask;
pub const K_CONTROL_MASK: Modifier = librime_sys2::RimeModifier_kControlMask;
pub const K_ALT_MASK: Modifier = librime_sys2::RimeModifier_kAltMask;
pub const K_RELEASE_MASK: Modifier = librime_sys2::RimeModifier_kReleaseMask;

pub const VK_PRIOR: u16 = 0x21;
pub const VK_NEXT: u16 = 0x22;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;
pub const VK_LEFT: u16 = 0x25;
pub const VK_UP: u16 = 0x26;
pub const VK_RIGHT: u16 = 0x27;
pub const VK_DOWN: u16 = 0x28;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_ESCAPE: u16 = 0x1B;
pub const VK_SPACE: u16 = 0x20;
pub const VK_DELETE: u16 = 0x2E;
pub const VK_CAPITAL: u16 = 0x14;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_OEM_1: u16 = 0xBA;
pub const VK_OEM_7: u16 = 0xDE;
pub const VK_OEM_4: u16 = 0xDB;
pub const VK_OEM_6: u16 = 0xDD;
pub const VK_OEM_COMMA: u16 = 0xBC;
pub const VK_OEM_PERIOD: u16 = 0xBE;
pub const VK_OEM_MINUS: u16 = 0xBD;
pub const VK_OEM_PLUS: u16 = 0xBB;
pub const VK_OEM_2: u16 = 0xBF;
pub const VK_OEM_5: u16 = 0xDC;
pub const VK_OEM_3: u16 = 0xC0;

// 小键盘：VK_NUMPAD0..9 是 0x60..=0x69，和 ASCII 小写字母区间**数值重叠**。
pub const VK_NUMPAD0: u16 = 0x60;
pub const VK_NUMPAD9: u16 = 0x69;
pub const VK_MULTIPLY: u16 = 0x6A;
pub const VK_ADD: u16 = 0x6B;
pub const VK_SUBTRACT: u16 = 0x6D;
pub const VK_DECIMAL: u16 = 0x6E;
pub const VK_DIVIDE: u16 = 0x6F;

// X11 小键盘 keysym（rime 的 key_binder 用 `KP_0`/`KP_Divide` 这些名字）。
pub const XK_KP_0: i32 = 0xFFB0;
pub const XK_KP_DECIMAL: i32 = 0xFFAE;
pub const XK_KP_DIVIDE: i32 = 0xFFAF;
pub const XK_KP_MULTIPLY: i32 = 0xFFAA;
pub const XK_KP_ADD: i32 = 0xFFAB;
pub const XK_KP_SUBTRACT: i32 = 0xFFAD;

pub const XK_SEMICOLON: i32 = 0x3B;
pub const XK_APOSTROPHE: i32 = 0x27;
pub const XK_BRACKETLEFT: i32 = 0x5B;
pub const XK_BRACKETRIGHT: i32 = 0x5D;
pub const XK_COMMA: i32 = 0x2C;
pub const XK_PERIOD: i32 = 0x2E;
pub const XK_MINUS: i32 = 0x2D;
pub const XK_EQUAL: i32 = 0x3D;
pub const XK_SLASH: i32 = 0x2F;
pub const XK_BACKSLASH: i32 = 0x5C;
pub const XK_GRAVE: i32 = 0x60;
pub const XK_CAPS_LOCK: i32 = 65509;

// 0x61..=0x7A 这段**不是**字母：字母的 VK 是 0x41..=0x5A（大写区），
// 0x60..=0x69 是小键盘 0..9。之前把 0x61..=0x7A 当字母放在前面，导致
// 小键盘 1 → 'a'、小键盘 /（0x6F，未覆盖）→ 'o' 这类错位（用户实测）。
pub fn vk_to_xk(vk: u16) -> i32 {
    match vk {
        VK_BACK => XK_BACK_SPACE as i32,
        VK_TAB => XK_TAB as i32,
        VK_RETURN => XK_RETURN as i32,
        VK_ESCAPE => XK_ESCAPE as i32,
        VK_SPACE => XK_SPACE as i32,
        VK_DELETE => XK_DELETE as i32,
        VK_PRIOR => XK_PRIOR as i32,
        VK_NEXT => XK_NEXT as i32,
        VK_HOME => XK_HOME as i32,
        VK_END => XK_END as i32,
        VK_LEFT => XK_LEFT as i32,
        VK_UP => XK_UP as i32,
        VK_RIGHT => XK_RIGHT as i32,
        VK_DOWN => XK_DOWN as i32,
        VK_CAPITAL => XK_CAPS_LOCK,
        VK_OEM_1 => XK_SEMICOLON,
        VK_OEM_7 => XK_APOSTROPHE,
        VK_OEM_4 => XK_BRACKETLEFT,
        VK_OEM_6 => XK_BRACKETRIGHT,
        VK_OEM_COMMA => XK_COMMA,
        VK_OEM_PERIOD => XK_PERIOD,
        VK_OEM_MINUS => XK_MINUS,
        VK_OEM_PLUS => XK_EQUAL,
        VK_OEM_2 => XK_SLASH,
        VK_OEM_5 => XK_BACKSLASH,
        VK_OEM_3 => XK_GRAVE,
        // 小键盘必须先于任何"数值区间"分支判断（见上面的注释）。
        VK_NUMPAD0..=VK_NUMPAD9 => XK_KP_0 + (vk - VK_NUMPAD0) as i32,
        VK_DECIMAL => XK_KP_DECIMAL,
        VK_DIVIDE => XK_KP_DIVIDE,
        VK_MULTIPLY => XK_KP_MULTIPLY,
        VK_ADD => XK_KP_ADD,
        VK_SUBTRACT => XK_KP_SUBTRACT,
        0x41..=0x5A => XK_A as i32 + (vk - 0x41) as i32,
        0x30..=0x39 => XK_0 as i32 + (vk - 0x30) as i32,
        _ => vk as i32,
    }
}

#[cfg(target_os = "windows")]
pub fn get_key_modifiers(is_key_up: bool) -> i32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState};

    unsafe {
        let mut modifiers = 0i32;
        if GetAsyncKeyState(VK_SHIFT as i32) < 0 {
            modifiers |= K_SHIFT_MASK as i32;
        }
        if GetAsyncKeyState(VK_CONTROL as i32) < 0 {
            modifiers |= K_CONTROL_MASK as i32;
        }
        if GetAsyncKeyState(VK_MENU as i32) < 0 {
            modifiers |= K_ALT_MASK as i32;
        }
        if GetKeyState(VK_CAPITAL as i32) & 0x01 != 0 {
            modifiers |= K_LOCK_MASK as i32;
        }
        if is_key_up {
            modifiers |= K_RELEASE_MASK as i32;
        }
        modifiers
    }
}

#[cfg(not(target_os = "windows"))]
pub fn get_key_modifiers(_is_key_up: bool) -> i32 {
    0
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key_code: KeyCode,
    pub modifiers: Modifier,
}

impl KeyEvent {
    pub fn new(key_code: KeyCode, modifiers: Modifier) -> Self {
        Self {
            key_code,
            modifiers,
        }
    }

    pub fn from_char(c: char) -> Self {
        let key_code = match c {
            'a'..='z' => XK_A + (c as KeyCode - 'a' as KeyCode),
            'A'..='Z' => XK_A + (c as KeyCode - 'A' as KeyCode),
            '0'..='9' => XK_0 + (c as KeyCode - '0' as KeyCode),
            ' ' => XK_SPACE,
            _ => c as KeyCode,
        };
        Self {
            key_code,
            modifiers: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 小键盘不能落到字母/数字区间里（0x60..=0x69 与 ASCII 小写字母数值重叠）。
    ///
    /// 实测症状：小键盘 1 打出 `a`、小键盘 `/` 打出 `o`（后者连分支都没命中，
    /// 走 `_ => vk` 变成了 ASCII 0x6F）。
    #[test]
    fn numpad_maps_to_kp_keysyms() {
        assert_eq!(vk_to_xk(0x61), XK_KP_0 + 1); // VK_NUMPAD1
        assert_eq!(vk_to_xk(0x69), XK_KP_0 + 9); // VK_NUMPAD9
        assert_eq!(vk_to_xk(0x60), XK_KP_0); // VK_NUMPAD0
        assert_eq!(vk_to_xk(0x6F), XK_KP_DIVIDE); // /
        assert_eq!(vk_to_xk(0x6A), XK_KP_MULTIPLY); // *
        assert_eq!(vk_to_xk(0x6B), XK_KP_ADD); // +
        assert_eq!(vk_to_xk(0x6D), XK_KP_SUBTRACT); // -
        assert_eq!(vk_to_xk(0x6E), XK_KP_DECIMAL); // .
    }

    /// 字母仍然按大写 VK 区（0x41..=0x5A）映射，主键盘不受影响。
    #[test]
    fn letters_still_map_from_uppercase_vk() {
        assert_eq!(vk_to_xk(0x41), XK_A as i32); // 'A'
        assert_eq!(vk_to_xk(0x5A), XK_A as i32 + 25); // 'Z'
        assert_eq!(vk_to_xk(0x31), XK_1 as i32); // 主键盘 '1'
    }

    #[test]
    fn test_keyevent_new() {
        let event = KeyEvent::new(XK_A, K_CONTROL_MASK);
        assert_eq!(event.key_code, XK_A);
        assert_eq!(event.modifiers, K_CONTROL_MASK);
    }

    #[test]
    fn test_keyevent_from_char_lowercase_a() {
        let event = KeyEvent::from_char('a');
        assert_eq!(event.key_code, XK_A);
        assert_eq!(event.modifiers, 0);
    }

    #[test]
    fn test_keyevent_from_char_lowercase_z() {
        let event = KeyEvent::from_char('z');
        assert_eq!(event.key_code, XK_Z);
        assert_eq!(event.modifiers, 0);
    }

    #[test]
    fn test_keyevent_from_char_uppercase_a() {
        let event = KeyEvent::from_char('A');
        assert_eq!(event.key_code, XK_A);
        assert_eq!(event.modifiers, 0);
    }

    #[test]
    fn test_keyevent_from_char_digits() {
        for d in '0'..='9' {
            let event = KeyEvent::from_char(d);
            assert_eq!(event.key_code, XK_0 + (d as KeyCode - '0' as KeyCode));
            assert_eq!(event.modifiers, 0);
        }
    }

    #[test]
    fn test_keyevent_from_char_space() {
        let event = KeyEvent::from_char(' ');
        assert_eq!(event.key_code, XK_SPACE);
        assert_eq!(event.modifiers, 0);
    }

    #[test]
    fn test_keycode_constants() {
        assert_eq!(XK_BACK_SPACE, librime_sys2::RimeKeyCode_XK_BackSpace);
        assert_eq!(XK_TAB, librime_sys2::RimeKeyCode_XK_Tab);
        assert_eq!(XK_RETURN, librime_sys2::RimeKeyCode_XK_Return);
        assert_eq!(XK_ESCAPE, librime_sys2::RimeKeyCode_XK_Escape);
        assert_eq!(XK_DELETE, librime_sys2::RimeKeyCode_XK_Delete);
        assert_eq!(XK_SPACE, librime_sys2::RimeKeyCode_XK_space);
    }

    /// 字母只来自大写 VK 区；0x61 是**小键盘 1**，不是 'a'（见 `numpad_maps_to_kp_keysyms`）。
    #[test]
    fn test_vk_to_xk_letters_uppercase_range() {
        assert_eq!(vk_to_xk(0x41), XK_A as i32);
        assert_eq!(vk_to_xk(0x5A), XK_A as i32 + 25);
    }

    #[test]
    fn test_vk_to_xk_oem_punctuation() {
        assert_eq!(vk_to_xk(VK_OEM_1), XK_SEMICOLON);
        assert_eq!(vk_to_xk(VK_OEM_7), XK_APOSTROPHE);
        assert_eq!(vk_to_xk(VK_OEM_4), XK_BRACKETLEFT);
        assert_eq!(vk_to_xk(VK_OEM_6), XK_BRACKETRIGHT);
        assert_eq!(vk_to_xk(VK_OEM_COMMA), XK_COMMA);
        assert_eq!(vk_to_xk(VK_OEM_PERIOD), XK_PERIOD);
        assert_eq!(vk_to_xk(VK_OEM_MINUS), XK_MINUS);
        assert_eq!(vk_to_xk(VK_OEM_PLUS), XK_EQUAL);
        assert_eq!(vk_to_xk(VK_OEM_2), XK_SLASH);
        assert_eq!(vk_to_xk(VK_OEM_5), XK_BACKSLASH);
        assert_eq!(vk_to_xk(VK_OEM_3), XK_GRAVE);
        assert_eq!(vk_to_xk(VK_CAPITAL), XK_CAPS_LOCK);
    }

    #[test]
    fn test_vk_to_xk_misc() {
        assert_eq!(vk_to_xk(VK_TAB), XK_TAB as i32);
        assert_eq!(vk_to_xk(VK_SPACE), XK_SPACE as i32);
        assert_eq!(vk_to_xk(0x31), XK_1 as i32);
    }

    #[test]
    fn test_modifier_constants() {
        assert_eq!(K_SHIFT_MASK, librime_sys2::RimeModifier_kShiftMask);
        assert_eq!(K_CONTROL_MASK, librime_sys2::RimeModifier_kControlMask);
        assert_eq!(K_ALT_MASK, librime_sys2::RimeModifier_kAltMask);
        assert_eq!(K_RELEASE_MASK, librime_sys2::RimeModifier_kReleaseMask);
    }
}
