// Souveraine boot bloom — Act IV+V, the quickshell half of the boot animation.
//
// The C splash (bootsplash/shader.frag) plays Act I-III on bare GPU: cursor
// blink, slide, and types "Hello There..." / "I'm Souveraine...", fading the
// lines out at ~6.8s. It then releases DRM master. Hyprland takes over and
// this ShaderEffect resumes the animation seamlessly — Souvie fades in and the
// bloom flower grows — over the live compositor, until the lockscreen is ready.
//
// This is the SAME flower as the C shader's flowerLayer/textLayer Souvie half,
// ported to Qt RHI (std140 UBO, qsb-compiled). The intro timeline is shifted so
// t=0 here corresponds to the C shader's t=CURSOR_SLIDE_END (3.0s): the caller
// feeds u_time already offset by the boot epoch so phase is continuous across
// the process swap. Everything is additive light on black.
//
// Qt RHI conventions vs raw GLES2:
//   - qt_TexCoord0 in [0,1], origin top-left; we build the same centered,
//     aspect-corrected uv the C shader derived from gl_FragCoord.
//   - uniforms live in a std140 ubuf block matching ShaderEffect properties.
//   - output is fragColor, premultiplied-alpha not needed (opaque black bg).

#version 440

layout(location = 0) in vec2 qt_TexCoord0;
layout(location = 0) out vec4 fragColor;

layout(std140, binding = 0) uniform buf {
    mat4  qt_Matrix;
    float qt_Opacity;
    float u_time;        // seconds since FLOWER epoch start (boot-aligned)
    vec2  u_resolution;  // px
    float u_fade;        // 1.0 = full, 0.0 = black (dismiss fade)
};

#define PI  3.14159265359

// The C shader's absolute timeline (seconds from boot t0). We keep the SAME
// numbers so the phase matches: the caller passes u_time in that same clock.
#define SOUVIE_FADE   vec2(7.4, 8.6)
#define FLOWER_START  8.0
#define BLOOM_DURATION 7.0

float easeOutBack(float t) {  // overshoots past 1 on purpose
    float c = 1.70158;
    float u = t - 1.0;
    return 1.0 + (c + 1.0) * u * u * u + c * u * u;
}

// Layered radial bloom on black — outer petals open first, inner follow.
// Verbatim port of flowerLayer() from bootsplash/shader.frag.
vec3 flowerLayer(vec2 p, float tF) {
    vec2 c = vec2(0.0, 0.08);
    vec2 q = p - c;
    q.y /= 0.82;
    float rho = length(q);
    float th = atan(q.y, q.x);

    float prog = clamp(tF / BLOOM_DURATION, 0.0, 1.0);
    float bloom = easeOutBack(prog);
    float gate = smoothstep(0.9, 1.0, prog);
    float breathe = 1.0 + gate * 0.02 * sin(tF * 0.8);

    vec3 acc = vec3(0.0);
    float maxR = 0.30 * breathe;

    for (int l = 0; l < 6; l++) {
        float lr = (float(l) + 0.5) / 6.0;
        float delay = (1.0 - lr) * 0.25;
        float lb = clamp((bloom - delay) / (1.0 - delay), 0.0, 1.0);
        if (lb < 0.01) continue;

        float n = 7.0 + float(l) * 2.0;
        float rotl = float(l) * 0.37 + gate * 0.06 * sin(tF * 0.5 + float(l));
        float lobe = pow(abs(cos(n * (th - rotl) * 0.5)), 3.0 - lr * 1.5);

        float d0 = maxR * max(lr, 0.12) * lb * 0.35;
        float plen = maxR * max(lr, 0.12) * lb * (0.9 - lr * 0.25);
        float reach = d0 + plen * (0.30 + 0.70 * lobe);

        float body = smoothstep(reach, reach * 0.55, rho)
                   * smoothstep(d0 * 0.6, d0 + 0.01, rho);
        float rim = exp(-abs(rho - reach) * 55.0) * lobe;

        float shimmer = 0.88 + 0.12 * sin(tF * 2.0 + sin(th * 13.0) * 5.0
                                          + rho * 40.0 + float(l) * 1.7);

        vec3 colL = mix(vec3(0.78, 0.12, 0.31), vec3(1.00, 0.51, 0.78), lr);
        colL = mix(colL, vec3(0.71, 0.40, 0.94), 0.35 * lr);

        acc += colL * (body * 0.22 + rim * 0.85) * lb * shimmer;
    }

    // starburst core
    float coreIn = smoothstep(0.25, 0.65, bloom);
    float rays = pow(abs(cos(4.0 * th + tF * 0.3)), 10.0);
    float core = exp(-rho * 20.0) * 1.3 + rays * exp(-rho * 8.0) * 0.45;
    acc += vec3(0.95, 0.86, 0.98) * core * coreIn;

    // ring glints
    if (bloom > 0.5) {
        float sb = min((bloom - 0.5) / 0.4, 1.0);
        float ring = exp(-abs(rho - maxR * 0.85 * sb) * 60.0);
        float glint = pow(abs(cos(8.0 * th + sin(tF * 0.5) * 0.4)), 24.0);
        acc += vec3(0.78, 0.24, 0.39) * ring * glint * sb * 0.8;
    }

    // stem
    float stemIn = clamp((bloom - 0.05) / 0.5, 0.0, 1.0);
    float stemTop = c.y - maxR * 0.15;
    float stemLen = 0.55 * stemIn;
    if (p.y < stemTop && p.y > stemTop - stemLen) {
        float st = (stemTop - p.y) / 0.55;
        float curveX = sin(st * PI * 0.3) * 0.045 + sin(st * PI * 0.8) * 0.015;
        float d = abs(p.x - (c.x + curveX));
        float shade = 0.55 + (1.0 - st) * 0.45;
        acc += vec3(0.16, 0.30, 0.10) * exp(-d * 220.0) * shade;
    }

    return acc;
}

void main() {
    float tt = u_time;

    // Rebuild the C shader's centered, y-normalized uv from qt_TexCoord0.
    // C: uv = (2*fragCoord - res) / res.y, origin bottom-left.
    // Qt: qt_TexCoord0 origin top-left, so flip y.
    vec2 frag = vec2(qt_TexCoord0.x, 1.0 - qt_TexCoord0.y) * u_resolution;
    vec2 uv = (2.0 * frag - u_resolution) / u_resolution.y;

    vec3 col = vec3(0.0);

    float fadeT = smoothstep(FLOWER_START, FLOWER_START + 0.8, tt);
    if (fadeT > 0.001)
        col += flowerLayer(uv, tt - FLOWER_START) * fadeT;

    // Souvie is drawn as a real PNG Image in the QML layer above this shader
    // (BootBloom.qml), not baked into the atlas — so the shader is bloom-only.

    fragColor = vec4(col * u_fade, 1.0);
}
