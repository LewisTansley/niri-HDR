// Blend-space transform for HDR outputs (KWin-aligned mixed SDR/HDR).
//
// Uniforms (defaults 0 = SDR passthrough):
//   niri_hdr_pq          — 1.0 enables HDR blend-space transforms
//   niri_ref_lum_scale   — reference_luminance / 10000 (paper white / PQ match)
//   niri_max_nit_scale   — max_nits / 10000 (display peak)
//   niri_sdr_lum_scale   — sdr_brightness / 10000 (SDR encode only; 0 = use ref)
//   niri_content_hdr     — 1.0 = source is PQ/BT.2020
//   niri_content_peak    — content max luminance / 10000 (0 = skip tonemap)
//   niri_linear          — >0.5 = extended-linear / scRGB
//   niri_linear_scale    — reference white / 10000 for linear content (1.0 signal)
//   niri_hdr_to_sdr      — 1.0 = PQ blend → electrical sRGB (capture)

uniform float niri_hdr_pq;
uniform float niri_ref_lum_scale;
uniform float niri_max_nit_scale;
uniform float niri_sdr_lum_scale;
uniform float niri_content_hdr;
uniform float niri_content_peak;
uniform float niri_linear;
uniform float niri_linear_scale;
uniform float niri_hdr_to_sdr;

vec3 niri_pq_oetf(vec3 lin) {
    const float m1 = 0.1593017578125;
    const float m2 = 78.84375;
    const float c1 = 0.8359375;
    const float c2 = 18.8515625;
    const float c3 = 18.6875;
    vec3 y = pow(max(lin, vec3(0.0)), vec3(m1));
    return pow((c1 + c2 * y) / (1.0 + c3 * y), vec3(m2));
}

vec3 niri_pq_eotf(vec3 pq) {
    const float m1 = 0.1593017578125;
    const float m2 = 78.84375;
    const float c1 = 0.8359375;
    const float c2 = 18.8515625;
    const float c3 = 18.6875;
    vec3 p = pow(clamp(pq, 0.0, 1.0), vec3(1.0 / m2));
    vec3 n = max(p - vec3(c1), vec3(0.0));
    vec3 d = max(vec3(c2) - c3 * p, vec3(1e-6));
    return pow(n / d, vec3(1.0 / m1));
}

// Rec. ITU-R BT.2100 ICtCp (BT.2020 RGB, PQ on LMS).
vec3 niri_bt2020_to_ictcp(vec3 rgb) {
    float l = (1688.0 * rgb.r + 2146.0 * rgb.g + 262.0 * rgb.b) / 4096.0;
    float m = (683.0 * rgb.r + 2951.0 * rgb.g + 462.0 * rgb.b) / 4096.0;
    float s = (99.0 * rgb.r + 309.0 * rgb.g + 3688.0 * rgb.b) / 4096.0;
    vec3 lms = niri_pq_oetf(max(vec3(l, m, s), vec3(0.0)));
    float i = 0.5 * lms.x + 0.5 * lms.y;
    float ct = (6610.0 * lms.x - 13613.0 * lms.y + 7003.0 * lms.z) / 4096.0;
    float cp = (17933.0 * lms.x - 17390.0 * lms.y - 543.0 * lms.z) / 4096.0;
    return vec3(i, ct, cp);
}

vec3 niri_ictcp_to_bt2020(vec3 ictcp) {
    float i = ictcp.x;
    float ct = ictcp.y;
    float cp = ictcp.z;
    float l = i + (0.00860903703793276) * ct + (0.111029625969183) * cp;
    float m = i - (0.00860903703793276) * ct - (0.111029625969183) * cp;
    float s = i + (0.560031335710679) * ct - (0.320627174987319) * cp;
    vec3 lms = niri_pq_eotf(vec3(l, m, s));
    // Inverse of the BT.2100 LMS matrix.
    float r = 3.436606 * lms.x - 2.506452 * lms.y + 0.069845 * lms.z;
    float g = -0.791330 * lms.x + 1.983600 * lms.y - 0.192271 * lms.z;
    float b = -0.025950 * lms.x - 0.098914 * lms.y + 1.124864 * lms.z;
    return vec3(r, g, b);
}

float niri_tonemap_i(float intensity, float ref_i, float peak_i, float max_i) {
    if (peak_i <= max_i + 1e-6 || intensity <= ref_i)
        return min(intensity, max_i);
    float t = clamp((intensity - ref_i) / max(peak_i - ref_i, 1e-6), 0.0, 1.0);
    float shaped = 1.0 - pow(1.0 - t, 1.5);
    return mix(ref_i, max_i, shaped);
}

// Pull RGB into [0, ∞) without inventing magenta: desaturate toward BT.2020 luminance
// until every channel is non-negative (high-chroma + compressed I often yields negatives).
vec3 niri_gamut_map_nonneg(vec3 rgb) {
    float y = dot(rgb, vec3(0.2627, 0.6780, 0.0593));
    float min_c = min(rgb.r, min(rgb.g, rgb.b));
    if (min_c >= 0.0)
        return rgb;
    if (y <= 1e-10)
        return vec3(0.0);
    float t = clamp(y / (y - min_c), 0.0, 1.0);
    return max(mix(vec3(y), rgb, t), vec3(0.0));
}

// Uniformly scale so max(R,G,B) ≤ max_scale — preserves hue unlike per-channel min().
vec3 niri_scale_to_peak(vec3 rgb, float max_scale) {
    float peak_c = max(rgb.r, max(rgb.g, rgb.b));
    return rgb * (max_scale / max(peak_c, max_scale));
}

vec3 niri_tonemap_bt2020(vec3 lin, float ref_scale, float max_scale, float content_peak_scale) {
    if (content_peak_scale <= max_scale + 1e-8)
        return lin;
    vec3 ictcp = niri_bt2020_to_ictcp(max(lin, vec3(0.0)));
    float ref_i = niri_pq_oetf(vec3(ref_scale)).x;
    float max_i = niri_pq_oetf(vec3(max_scale)).x;
    float peak_i = niri_pq_oetf(vec3(content_peak_scale)).x;
    ictcp.x = niri_tonemap_i(ictcp.x, ref_i, peak_i, max_i);
    return niri_gamut_map_nonneg(niri_ictcp_to_bt2020(ictcp));
}

vec4 niri_blend(vec4 color) {
    if (niri_hdr_to_sdr > 0.5) {
        float a = color.a;
        vec3 rgb = a > 0.0 ? color.rgb / a : color.rgb;
        rgb = niri_pq_eotf(rgb);
        const mat3 to_bt709 = mat3(
            1.660491, -0.124550, -0.018151,
           -0.587641,  1.132900, -0.100579,
           -0.072850, -0.008349,  1.118730);
        rgb = to_bt709 * rgb;
        float ref_scale = niri_ref_lum_scale > 0.0 ? niri_ref_lum_scale : 0.0203;
        rgb = clamp(rgb / ref_scale, 0.0, 1.0);
        rgb = pow(rgb, vec3(1.0 / 2.2));
        return vec4(rgb * a, a);
    }

    if (niri_hdr_pq < 0.5 && niri_linear < 0.5)
        return color;

    float a = color.a;
    vec3 rgb = a > 0.0 ? color.rgb / a : color.rgb;

    const mat3 to_bt2020 = mat3(
        0.627404, 0.069097, 0.016391,
        0.329283, 0.919540, 0.088013,
        0.043313, 0.011362, 0.895595);

    float ref_scale = niri_ref_lum_scale > 0.0 ? niri_ref_lum_scale : 0.0203;
    float max_scale = niri_max_nit_scale > 0.0 ? niri_max_nit_scale : max(ref_scale, 0.05);

    if (niri_linear > 0.5) {
        float lin_scale = niri_linear_scale > 0.0 ? niri_linear_scale : ref_scale;
        if (niri_hdr_pq > 0.5) {
            rgb = to_bt2020 * rgb;
            // scRGB can be slightly negative out-of-gamut; desaturate instead of zeroing a channel.
            rgb = niri_gamut_map_nonneg(rgb) * lin_scale;
            // Tonemap only when content peak exceeds the display (same rule as the PQ path).
            // Do not floor peak to 4000 nits — that forced ICtCp crush on every scRGB game
            // even when content/display are ~1000 nits (Windows-scRGB carries no max_cll).
            if (niri_content_peak > max_scale) {
                rgb = niri_tonemap_bt2020(rgb, ref_scale, max_scale, niri_content_peak);
            }
            // Luminance-preserving peak limit (not per-channel min — that blooms pink/magenta).
            rgb = niri_scale_to_peak(rgb, max_scale);
            rgb = niri_pq_oetf(rgb);
            return vec4(rgb * a, a);
        }
        rgb = clamp(rgb * (lin_scale / ref_scale), 0.0, 1.0);
        rgb = pow(max(rgb, vec3(0.0)), vec3(1.0 / 2.2));
        return vec4(rgb * a, a);
    }

    if (niri_content_hdr > 0.5) {
        // PQ content: decode, match PQ reference white (203 nits) to paper white, tonemap.
        rgb = niri_pq_eotf(rgb);
        const float pq_ref = 0.0203;
        rgb *= (ref_scale / pq_ref);
        if (niri_content_peak > max_scale) {
            rgb = niri_tonemap_bt2020(rgb, ref_scale, max_scale, niri_content_peak);
        }
        // A raised reference-luminance or bad max_cll must not push the encode past 1.0.
        rgb = niri_scale_to_peak(max(rgb, vec3(0.0)), max_scale);
        rgb = niri_pq_oetf(rgb);
        return vec4(rgb * a, a);
    }

    // SDR → HDR container (paper-white encode / desktop inverse tonemap).
    // Uses sdr_brightness when set; does not affect PQ or scRGB paths above.
    float sdr_scale = niri_sdr_lum_scale > 0.0 ? niri_sdr_lum_scale : ref_scale;
    // SDR is defined in [0, 1]. Extended-range values here mean a mis-tagged buffer, and
    // gamma-expanding them overflows the PQ encode, which clips per channel and breaks hue.
    rgb = pow(clamp(rgb, 0.0, 1.0), vec3(2.2));
    rgb = to_bt2020 * rgb;
    rgb = niri_pq_oetf(niri_scale_to_peak(rgb * sdr_scale, max_scale));
    return vec4(rgb * a, a);
}
