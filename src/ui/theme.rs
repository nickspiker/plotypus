// Global theme colours and constants
// All colours are u32 in packed ARGB format: 0xAARRGGBB
// On Android, colours are converted to ABGR at compile-time via fmt()

/// Compile-time colour format conversion for Android (ARGB → ABGR)
#[cfg(target_os = "android")]
const fn fmt(argb: u32) -> u32 {
    let a = (argb >> 24) & 0xFF;
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    (a << 24) | (b << 16) | (g << 8) | r
}

/// Identity function on non-Android platforms
#[cfg(not(target_os = "android"))]
const fn fmt(argb: u32) -> u32 {
    argb
}

// Window edge colours
pub const WINDOW_LIGHT_EDGE: u32 = fmt(0xFF_44_41_37);
pub const WINDOW_SHADOW_EDGE: u32 = fmt(0xFF_2B_34_37);
pub const WINDOW_CONTROLS_BG: u32 = fmt(0xFF_1E_1E_1E); // Background behind window control buttons
pub const WINDOW_CONTROLS_HAIRLINE: u32 = fmt(0xFF_44_41_37); // Hairline separators between buttons

// UI element colours
pub const LIGHT_EDGE: u32 = fmt(0xFF_60_60_60);
pub const SHADOW_EDGE: u32 = fmt(0xFF_20_20_20);
pub const FILL: u32 = fmt(0xFF_40_40_40);

// Text colours
pub const LABEL_COLOUR: u32 = fmt(0xFF_80_80_80);
pub const CURSOR_BRIGHTNESS: f32 = 100.; // Cursor wave brightness multiplier
pub const TEXT_COLOUR: u32 = fmt(0xFF_D0_D0_D0);
pub const TEXT_SELECTION_COLOUR: u32 = fmt(0xFF_D0_D0_D0);

// Button colours
pub const BUTTON_BASE: u32 = fmt(0xFF_40_40_40);
pub const BUTTON_LIGHT_EDGE: u32 = fmt(0xFF_60_60_60);
pub const BUTTON_SHADOW_EDGE: u32 = fmt(0xFF_20_20_20);
pub const BUTTON_HAIRLINE: u32 = fmt(0xFF_32_32_32); // Hairline separators between buttons
pub const BUTTON_BLUE: u32 = fmt(0xFF_20_30_50);
pub const BUTTON_GREEN: u32 = fmt(0xFF_20_45_25);
pub const BUTTON_YELLOW: u32 = fmt(0xFF_50_45_20);

// Button glyphs (base colours, not deltas)
pub const CLOSE_GLYPH: u32 = fmt(0xFF_80_20_20);
pub const MAXIMIZE_GLYPH: u32 = fmt(0xFF_48_6B_3A);
pub const MAXIMIZE_GLYPH_INTERIOR: u32 = fmt(0xFF_28_2D_2E);
pub const MINIMIZE_GLYPH: u32 = fmt(0xFF_33_30_C7);

// Button hover deltas (applied on hover, negated on unhover)
// RGB channels wrap intentionally; 0xFF alpha absorbs carry from R overflow
pub const CLOSE_HOVER: u32 = fmt(0xFF_21_FD_F9); // Red
pub const MAXIMIZE_HOVER: u32 = fmt(0xFF_FA_10_FA); // Green
pub const MINIMIZE_HOVER: u32 = fmt(0xFF_F7_FA_25); // Blue
pub const TEXTBOX_HOVER: u32 = fmt(0x00_0A_0A_0A); // Suttle brightness increase
pub const QUERY_BUTTON_HOVER: u32 = fmt(0x00_0F_0F_0F); // Brighter than textbox
pub const BACK_HEADER_HOVER: u32 = fmt(0x00_0C_0C_0C); // Suttle brightness for header

// Textbox colours
pub const TEXTBOX_LIGHT_EDGE: u32 = fmt(0xFF_44_41_37);
pub const TEXTBOX_SHADOW_EDGE: u32 = fmt(0xFF_2B_34_37);
pub const TEXTBOX_FILL: u32 = fmt(0xFF_06_08_09);

// Logo colours (grayscale for glow/highlight)
pub const LOGO_GLOW_GRAY: u8 = 192; // Logo glow effect (grayscale)
pub const LOGO_HIGHLIGHT_GRAY: u8 = 128; // Logo highlight (grayscale)
pub const LOGO_TEXT: u32 = fmt(0xFF_00_00_00); // Logo text colour

// Contact list colours
pub const CONTACT_NAME: u32 = fmt(0xFF_F0_F0_F0); // Contact name (white)
pub const CONTACT_NAME_UNHOVERED: u32 = fmt(0xFF_A0_A0_A0); // Contact name when not hovered
pub const VERSION_TEXT: u32 = fmt(0xFF_66_66_66); // Version number at bottom (dim grey)
pub const ONLINE_DOT: u32 = fmt(0x00_00_F0_00); // Self online status dot fill (no alpha, medium green)
pub const OFFLINE_DOT: u32 = fmt(0x00_80_80_80); // Offline status hairline ring (gray)
pub const CONTACT_ONLINE: u32 = fmt(0xFF_00_F0_00); // Online status indicator/back arrow (bright green)
pub const CONTACT_OFFLINE: u32 = fmt(0x00_40_40_40); // Offline status hairline ring (gray)
pub const CONTACT_HOVER_DELTA: u32 = fmt(0x00_5F_5F_5F); // Brightness delta for contact hover (0xFF - 0xA0)
pub const CONTACT_BRIGHTEN_DELTA: u32 = fmt(0x00_60_60_60); // Additive brightness for contact interaction

// Search result colours
pub const SEARCH_RESULT_ADDED: u32 = fmt(0xFF_40_FF_40); // "added {handle}" success (bright green)
pub const SEARCH_RESULT_NOT_FOUND: u32 = fmt(0xFF_FF_60_60); // "not found" error (red)

// Glow colours (0x00RRGGBB format - no alpha channel, applied to textbox glow)
pub const GLOW_DEFAULT: u32 = fmt(0x00_FF_FF_FF); // White glow (default state)
pub const GLOW_ATTESTING: u32 = fmt(0x00_FF_FF_40); // Yellow glow (attesting/searching)
pub const GLOW_SUCCESS: u32 = fmt(0x00_40_FF_40); // Green glow (search found)
pub const GLOW_ERROR: u32 = fmt(0x00_FF_60_60); // Red glow (error/not found)

// UI text colours
pub const BUTTON_TEXT: u32 = fmt(0xFF_D0_D0_D0); // Button text (send, attest, query)
pub const COUNTER_TEXT: u32 = fmt(0xFF_FF_FF_FF); // Frame counter text (white)
pub const PLACEHOLDER_TEXT: u32 = fmt(0xFF_80_80_80); // Placeholder symbol text (gray)
pub const STATUS_TEXT_ATTESTING: u32 = fmt(0xFF_FF_FF_00); // "Attesting..." status text (yellow)
pub const STATUS_TEXT_ERROR: u32 = fmt(0xFF_FF_00_00); // Error message text (red)
pub const ZOOM_HINT_TEXT: u32 = fmt(0xFF_80_80_80); // Zoom level hint text (gray)

// Debug colours
pub const DEBUG_MARKER: u32 = fmt(0xFE_FF_00_FF); // Magenta marker for debugging (alpha=254)

// Background texture colours
pub const BG_BASE: u32 = fmt(0xFF_0C_14_0E); // Base dark green for background
pub const BG_MASK: u32 = fmt(0xFF_0F_07_1F); // Channel mask for noise generation
pub const BG_ALPHA: u32 = fmt(0xFF_00_00_00); // Alpha channel (opaque)
pub const BG_SPECKLE: u32 = fmt(0x00_3F_1F_7F); // Speckle highlight colour (no alpha)

// Message colours
pub const MESSAGE_SENT: u32 = fmt(0xFF_FF_A0_40); // Orange for outgoing messages
pub const MESSAGE_RECEIVED: u32 = fmt(0xFF_40_E0_E0); // Cyan for incoming messages
pub const MESSAGE_INDICATOR_SENT: u32 = fmt(0xFF_C0_C0_C0); // Gray for "sent" indicator
pub const MESSAGE_INDICATOR_ACKD: u32 = fmt(0xFF_00_FF_00); // Green for "ACK'd" indicator

// Font families and weights
pub const FONT_LOGO: &str = "Oxanium";
pub const FONT_UI: &str = "Josefin Slab";
pub const FONT_USER_CONTENT: &str = "Open Sans";
pub const FONT_WEIGHT_USER_CONTENT: u16 = 400; // Font weight for user-entered text
