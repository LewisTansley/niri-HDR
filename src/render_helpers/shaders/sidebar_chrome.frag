precision highp float;

#if defined(DEBUG_FLAGS)
uniform float niri_tint;
#endif

uniform float niri_alpha;
uniform float niri_scale;

uniform vec2 niri_size;
varying vec2 niri_v_coords;

uniform mat3 input_to_geo;
uniform vec2 geo_size;
// Border / solid color when has_fill is 0, otherwise the border ring color.
uniform vec4 color;
// Interior fill when has_fill > 0.5; ignored otherwise.
uniform vec4 fill_color;
uniform float has_fill;
// 0 = left sidebar (wingtips on the right), 1 = right sidebar (wingtips on the left).
uniform float side;
uniform float scoop_radius;
// Panel body width before the R viewport extension.
uniform float panel_width;
// 0 = filled shape only; >0 = border ring width.
uniform float border_width;

float aa_outside_circle(vec2 p, vec2 center, float radius) {
    float dist = distance(p, center);
    float t = clamp((dist - radius) * niri_scale + 0.5, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// DMS goth wingtip silhouette: panel body + R×R brackets into the viewport.
float wingtip_coverage(vec2 p, vec2 size, float W, float R) {
    float H = size.y;
    if (p.x < 0.0 || p.y < 0.0 || p.x > size.x || p.y > H) {
        return 0.0;
    }

    R = min(max(R, 0.0), min(W, H) * 0.5);

    if (side < 0.5) {
        // Left sidebar: body [0,W]×[0,H], extension to the right.
        if (p.x <= W) {
            return 1.0;
        }
        if (R <= 0.0) {
            return 0.0;
        }
        if (p.y < R) {
            return aa_outside_circle(p, vec2(W + R, R), R);
        }
        if (p.y > H - R) {
            return aa_outside_circle(p, vec2(W + R, H - R), R);
        }
        return 0.0;
    } else {
        // Right sidebar: extension [0,R], body [R, R+W] = [R, size.x].
        float body_left = size.x - W;
        if (p.x >= body_left) {
            return 1.0;
        }
        if (R <= 0.0) {
            return 0.0;
        }
        if (p.y < R) {
            return aa_outside_circle(p, vec2(0.0, R), R);
        }
        if (p.y > H - R) {
            return aa_outside_circle(p, vec2(0.0, H - R), R);
        }
        return 0.0;
    }
}

// Shape inset by bw (parallel to wingtip silhouette) for hollow border interior.
float wingtip_inner(vec2 p, vec2 size, float W, float R, float bw) {
    float H = size.y;
    if (bw <= 0.0) {
        return wingtip_coverage(p, size, W, R);
    }

    R = min(max(R, 0.0), min(W, H) * 0.5);
    float rin = R + bw;

    if (side < 0.5) {
        if (p.x < bw || p.y < bw || p.y > H - bw || p.x > W + R) {
            return 0.0;
        }

        if (R <= 0.0) {
            // Inset rect of the panel body only.
            return (p.x <= W - bw) ? 1.0 : 0.0;
        }

        if (p.x <= W - bw) {
            if (p.y >= R && p.y <= H - R) {
                return 1.0;
            }
            if (p.y < R) {
                return aa_outside_circle(p, vec2(W + R, R), rin);
            }
            return aa_outside_circle(p, vec2(W + R, H - R), rin);
        }

        // Near workspace edge / extension.
        if (p.y < rin) {
            return aa_outside_circle(p, vec2(W + R, R), rin);
        }
        if (p.y > H - rin) {
            return aa_outside_circle(p, vec2(W + R, H - R), rin);
        }
        return 0.0;
    } else {
        float body_left = size.x - W;
        if (p.x > size.x - bw || p.y < bw || p.y > H - bw || p.x < 0.0) {
            return 0.0;
        }

        if (R <= 0.0) {
            return (p.x >= body_left + bw) ? 1.0 : 0.0;
        }

        if (p.x >= body_left + bw) {
            if (p.y >= R && p.y <= H - R) {
                return 1.0;
            }
            if (p.y < R) {
                return aa_outside_circle(p, vec2(0.0, R), rin);
            }
            return aa_outside_circle(p, vec2(0.0, H - R), rin);
        }

        if (p.y < rin) {
            return aa_outside_circle(p, vec2(0.0, R), rin);
        }
        if (p.y > H - rin) {
            return aa_outside_circle(p, vec2(0.0, H - R), rin);
        }
        return 0.0;
    }
}

vec4 niri_blend(vec4 color);

void main() {
    vec3 coords_geo = input_to_geo * vec3(niri_v_coords, 1.0);
    float W = panel_width;
    float R = scoop_radius;
    float outer = wingtip_coverage(coords_geo.xy, geo_size, W, R);

    vec4 out_color;
    if (border_width > 0.0 && has_fill > 0.5) {
        float inner = wingtip_inner(coords_geo.xy, geo_size, W, R, border_width);
        out_color = mix(color, fill_color, inner) * outer;
    } else if (border_width > 0.0) {
        float inner = wingtip_inner(coords_geo.xy, geo_size, W, R, border_width);
        out_color = color * (outer * (1.0 - inner));
    } else {
        vec4 solid = has_fill > 0.5 ? fill_color : color;
        out_color = solid * outer;
    }

    out_color = out_color * niri_alpha;

#if defined(DEBUG_FLAGS)
    if (niri_tint == 1.0)
        out_color = vec4(0.0, 0.2, 0.0, 0.2) + out_color * 0.8;
#endif

    gl_FragColor = niri_blend(out_color);
}
