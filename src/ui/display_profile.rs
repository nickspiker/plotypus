//! Display ICC profile parsing and color conversion
//!
//! Queries the display's ICC profile and converts VSF RGB to display colorspace.

use icc_profile::{Data, DecodedICCProfile, ICCNumber};
use vsf::colour::convert::apply_matrix_3x3_f32;
use vsf::colour::VSF_RGB2XYZ;

/// Display color converter
pub struct DisplayConverter {
    /// XYZ → Display RGB matrix (inverted from ICC profile)
    xyz_to_display: [f32; 9],
    /// TRC curves for each channel (linear → display gamma)
    r_trc: TrcCurve,
    g_trc: TrcCurve,
    b_trc: TrcCurve,
}

/// Tone Reproduction Curve (pre-inverted for OETF application)
#[derive(Clone)]
enum TrcCurve {
    Linear,
    Gamma(f32),
    /// Pre-inverted LUT: index by linear*255, get encoded u8 directly
    InvertedLut([u8; 256]),
    Parametric {
        function_type: u16,
        vals: Vec<f32>,
    },
}

impl DisplayConverter {
    /// Create converter by querying display ICC profile
    /// Falls back to sRGB if no profile found
    pub fn new() -> Self {
        if let Some(profile_bytes) = get_display_profile() {
            if let Ok(converter) = Self::from_icc_profile(&profile_bytes) {
                eprintln!("Display: Using ICC profile from system");
                return converter;
            }
        }

        eprintln!("Display: Using sRGB fallback");
        Self::srgb_fallback()
    }

    /// Parse ICC profile into converter
    fn from_icc_profile(icc_bytes: &[u8]) -> Result<Self, String> {
        let icc_vec = icc_bytes.to_vec();
        let profile = DecodedICCProfile::new(&icc_vec)
            .map_err(|e| format!("Failed to parse ICC profile: {:?}", e))?;

        // Extract RGB→XYZ matrix from rXYZ, gXYZ, bXYZ tags
        let r_xyz = extract_xyz(&profile, "rXYZ")?;
        let g_xyz = extract_xyz(&profile, "gXYZ")?;
        let b_xyz = extract_xyz(&profile, "bXYZ")?;

        // Build Display_RGB→XYZ matrix (column-major)
        let display_to_xyz = [
            r_xyz[0], r_xyz[1], r_xyz[2], g_xyz[0], g_xyz[1], g_xyz[2], b_xyz[0], b_xyz[1],
            b_xyz[2],
        ];

        // Invert to get XYZ→Display_RGB
        let xyz_to_display = invert_3x3(&display_to_xyz);

        // Parse TRC curves
        let r_trc = parse_trc(profile.tags.get("rTRC"))?;
        let g_trc = parse_trc(profile.tags.get("gTRC"))?;
        let b_trc = parse_trc(profile.tags.get("bTRC"))?;

        Ok(Self {
            xyz_to_display,
            r_trc,
            g_trc,
            b_trc,
        })
    }

    /// sRGB fallback when no ICC profile available
    fn srgb_fallback() -> Self {
        // XYZ→sRGB matrix (D65 white point)
        // This is the inverse of the sRGB→XYZ matrix
        let xyz_to_display = [
            3.2404542, -0.9692660, 0.0556434, -1.5371385, 1.8760108, -0.2040259, -0.4985314,
            0.0415560, 1.0572252,
        ];

        // sRGB uses gamma ~2.4 with linear toe
        let srgb_trc = TrcCurve::Parametric {
            function_type: 0x0003,
            vals: vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
        };

        Self {
            xyz_to_display,
            r_trc: srgb_trc.clone(),
            g_trc: srgb_trc.clone(),
            b_trc: srgb_trc,
        }
    }

    /// Convert VSF RGB pixels to display colorspace
    /// Input: 256×256×3 VSF RGB gamma-encoded bytes
    /// Output: 256×256×3 Display RGB bytes (BGR order on Android for ABGR surface)
    pub fn convert_avatar(&self, vsf_rgb: &[u8]) -> Vec<u8> {
        let pixel_count = 256 * 256;
        let mut output = vec![0u8; pixel_count * 3];

        for i in 0..pixel_count {
            let idx = i * 3;

            // VSF RGB gamma-encoded → linear (gamma 2 = square)
            let r_lin = (vsf_rgb[idx] as f32 / 256.).powi(2);
            let g_lin = (vsf_rgb[idx + 1] as f32 / 256.).powi(2);
            let b_lin = (vsf_rgb[idx + 2] as f32 / 256.).powi(2);

            // Linear VSF RGB → XYZ
            let xyz = apply_matrix_3x3_f32(&VSF_RGB2XYZ, &[r_lin, g_lin, b_lin]);

            // XYZ → Linear Display RGB
            let display_lin = apply_matrix_3x3_f32(&self.xyz_to_display, &xyz);

            // Apply TRC (linear → display gamma-encoded)
            let r = apply_trc(display_lin[0], &self.r_trc);
            let g = apply_trc(display_lin[1], &self.g_trc);
            let b = apply_trc(display_lin[2], &self.b_trc);

            // Android uses ABGR surface format, swap R↔B at load time
            // so draw_avatar's ARGB writes become ABGR
            #[cfg(target_os = "android")]
            {
                output[idx] = b;
                output[idx + 1] = g;
                output[idx + 2] = r;
            }
            #[cfg(not(target_os = "android"))]
            {
                output[idx] = r;
                output[idx + 1] = g;
                output[idx + 2] = b;
            }
        }

        output
    }
}

/// Extract XYZ values from ICC profile tag
fn extract_xyz(profile: &DecodedICCProfile, tag: &str) -> Result<[f32; 3], String> {
    match profile.tags.get(tag) {
        Some(Data::XYZNumber(xyz)) => Ok([xyz.x.as_f32(), xyz.y.as_f32(), xyz.z.as_f32()]),
        Some(Data::XYZNumberArray(arr)) if !arr.is_empty() => {
            let xyz = &arr[0];
            Ok([xyz.x.as_f32(), xyz.y.as_f32(), xyz.z.as_f32()])
        }
        _ => Err(format!("ICC profile missing {} tag", tag)),
    }
}

/// Parse TRC curve from ICC profile tag
fn parse_trc(trc: Option<&Data>) -> Result<TrcCurve, String> {
    match trc {
        Some(Data::Curve(curve)) => {
            if curve.is_empty() {
                Ok(TrcCurve::Linear)
            } else if curve.len() == 1 {
                let gamma = curve[0] as f32 / 256.;
                Ok(TrcCurve::Gamma(gamma))
            } else {
                // ICC TRC LUT maps encoded→linear (EOTF), pre-invert to linear→encoded
                let eotf: Vec<f32> = curve.iter().map(|&v| v as f32 / 65535.).collect();
                let mut inverted = [0u8; 256];

                for i in 0..256 {
                    let target_linear = i as f32 / 255.;

                    // Find encoded value that produces this linear (scan from last match)
                    let mut best_enc = 0u8;
                    for (enc, &lin) in eotf.iter().enumerate() {
                        if lin <= target_linear {
                            best_enc = (enc * 255 / (eotf.len() - 1)) as u8;
                        } else {
                            break;
                        }
                    }
                    inverted[i] = best_enc;
                }

                Ok(TrcCurve::InvertedLut(inverted))
            }
        }
        Some(Data::ParametricCurve(param)) => {
            let vals: Vec<f32> = param.vals.iter().map(|v| v.as_f32()).collect();
            Ok(TrcCurve::Parametric {
                function_type: param.funtion_type,
                vals,
            })
        }
        None => Ok(TrcCurve::Gamma(2.2)), // Fallback
        _ => Err("Unsupported TRC type".to_string()),
    }
}

/// Apply TRC curve to convert linear [0,1] to display u8
/// This applies the OETF (optical-electro transfer function)
fn apply_trc(linear: f32, trc: &TrcCurve) -> u8 {
    let clamped = linear.max(0.0).min(1.0);

    let encoded = match trc {
        TrcCurve::Linear => clamped,
        TrcCurve::Gamma(gamma) => clamped.powf(1.0 / gamma),
        TrcCurve::InvertedLut(lut) => {
            // Pre-inverted: index by linear, get encoded directly
            return lut[(clamped * 255.) as usize];
        }
        TrcCurve::Parametric {
            function_type,
            vals,
        } => {
            // ICC parametric curves - apply inverse for OETF
            match function_type {
                0x0000 => clamped.powf(1.0 / vals[0]),
                0x0001 => {
                    let gamma = vals[0];
                    let a = vals[1];
                    let b = vals[2];
                    // Inverse of Y = (aX + b)^gamma
                    (clamped.powf(1.0 / gamma) - b) / a
                }
                0x0003 => {
                    // sRGB-style: Y = (aX + b)^gamma if X >= d, else cX
                    // Inverse: X = ((Y)^(1/gamma) - b) / a if Y >= (ad+b)^gamma, else Y/c
                    let gamma = vals[0];
                    let a = vals[1];
                    let b = vals[2];
                    let c = vals[3];
                    let d = vals[4];
                    let threshold = (a * d + b).powf(gamma);
                    if clamped >= threshold {
                        (clamped.powf(1.0 / gamma) - b) / a
                    } else {
                        clamped / c
                    }
                }
                _ => clamped.powf(1.0 / 2.2), // Fallback
            }
        }
    };

    (encoded.max(0.0).min(1.0) * 256.) as u8
}

/// Invert a 3×3 matrix (column-major format)
fn invert_3x3(m: &[f32; 9]) -> [f32; 9] {
    // For column-major: m[col*3 + row]
    // m[0],m[1],m[2] = column 0
    // m[3],m[4],m[5] = column 1
    // m[6],m[7],m[8] = column 2

    let a = m[0];
    let b = m[3];
    let c = m[6];
    let d = m[1];
    let e = m[4];
    let f = m[7];
    let g = m[2];
    let h = m[5];
    let i = m[8];

    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);

    if det.abs() < 1e-10 {
        // Singular matrix - return identity
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }

    let inv_det = 1.0 / det;

    // Adjugate matrix elements
    let a11 = (e * i - f * h) * inv_det;
    let a12 = (c * h - b * i) * inv_det;
    let a13 = (b * f - c * e) * inv_det;
    let a21 = (f * g - d * i) * inv_det;
    let a22 = (a * i - c * g) * inv_det;
    let a23 = (c * d - a * f) * inv_det;
    let a31 = (d * h - e * g) * inv_det;
    let a32 = (b * g - a * h) * inv_det;
    let a33 = (a * e - b * d) * inv_det;

    // Return in column-major format
    [a11, a21, a31, a12, a22, a32, a13, a23, a33]
}

/// Query display ICC profile from system
#[cfg(target_os = "linux")]
fn get_display_profile() -> Option<Vec<u8>> {
    use std::ffi::CString;
    use std::ptr;

    unsafe {
        let display = x11::xlib::XOpenDisplay(ptr::null());
        if display.is_null() {
            eprintln!("Display: Failed to open X11 display");
            return None;
        }

        let atom_name = CString::new("_ICC_PROFILE").ok()?;
        let atom = x11::xlib::XInternAtom(display, atom_name.as_ptr(), 0);
        if atom == 0 {
            x11::xlib::XCloseDisplay(display);
            eprintln!("Display: _ICC_PROFILE atom not found");
            return None;
        }

        let root = x11::xlib::XDefaultRootWindow(display);

        let mut actual_type: x11::xlib::Atom = 0;
        let mut actual_format: i32 = 0;
        let mut nitems: u64 = 0;
        let mut bytes_after: u64 = 0;
        let mut prop: *mut u8 = ptr::null_mut();

        let result = x11::xlib::XGetWindowProperty(
            display,
            root,
            atom,
            0,        // offset
            i64::MAX, // length (get all)
            0,        // delete = false
            x11::xlib::AnyPropertyType as u64,
            &mut actual_type,
            &mut actual_format,
            &mut nitems,
            &mut bytes_after,
            &mut prop,
        );

        if result != 0 || prop.is_null() || nitems == 0 {
            x11::xlib::XCloseDisplay(display);
            eprintln!("Display: No _ICC_PROFILE property on root window");
            return None;
        }

        // Copy the profile data
        let profile_bytes = std::slice::from_raw_parts(prop, nitems as usize).to_vec();

        x11::xlib::XFree(prop as *mut _);
        x11::xlib::XCloseDisplay(display);

        eprintln!("Display: Got ICC profile, {} bytes", profile_bytes.len());
        Some(profile_bytes)
    }
}

#[cfg(target_os = "windows")]
fn get_display_profile() -> Option<Vec<u8>> {
    use windows::core::PWSTR;
    use windows::Win32::Graphics::Gdi::GetDC;
    use windows::Win32::UI::ColorSystem::GetICMProfileW;

    unsafe {
        let hdc = GetDC(None);
        if hdc.is_invalid() {
            return None;
        }

        // First call to get required buffer size
        let mut size: u32 = 0;
        let _ = GetICMProfileW(hdc, &mut size, PWSTR::null());

        if size == 0 {
            return None;
        }

        // Allocate buffer and get the path
        let mut path: Vec<u16> = vec![0; size as usize];
        if !GetICMProfileW(hdc, &mut size, PWSTR(path.as_mut_ptr())).as_bool() {
            return None;
        }

        // Convert to string and read file
        let path_str = String::from_utf16_lossy(&path[..size as usize - 1]);
        eprintln!("Display: ICC profile at {}", path_str);
        std::fs::read(&path_str).ok()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn get_display_profile() -> Option<Vec<u8>> {
    None
}
