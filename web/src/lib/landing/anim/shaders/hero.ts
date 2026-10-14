import { PRELUDE, SNOISE } from './common'

/**
 * The hero's background: three colour foci (brand palette) in a slowly flowing field. The field
 * is warped by simplex noise and displaced by the pointer's flow map, so the colours are dragged
 * along the cursor's wake and keep drifting after it. The preview panel (`P`: x, y, w, h in CSS
 * px, radius `PR`) is darkened here, since the particles are drawn over it.
 */
export const HERO = /* glsl */ `${PRELUDE}
uniform vec4 F[3];
uniform vec4 P;
uniform float PR;
const vec3 C0 = vec3(0.0, 193.0, 213.0) / 255.0;
const vec3 C1 = vec3(176.0, 96.0, 240.0) / 255.0;
const vec3 C2 = vec3(255.0, 122.0, 61.0) / 255.0;

void main() {
  vec2 p = v * S;
  float side = min(S.x, S.y);
  vec3 fl = flow(v, side * 0.3);
  // The fluid's own slow motion, then the wake the pointer leaves in it.
  vec2 n = (p + fl.xy * 0.5) / (side * 0.9);
  vec2 w = vec2(fbm(n + T * 0.04), fbm(n + vec2(5.2, 1.3) - T * 0.03));
  vec2 q = p + fl.xy + w * side * 0.2;
  vec3 acc = vec3(0.0);
  vec3 C[3] = vec3[3](C0, C1, C2);
  for (int i = 0; i < 3; i++) {
    vec2 c = F[i].xy * S;
    // Radii are fractions of the width, like the blurred gradients of the CSS fallback.
    float d = length(q - c) / (F[i].z * S.x);
    acc += C[i] * exp(-d * d * 2.4) * 0.95;
  }
  // The wake glows a little brighter.
  acc *= 1.0 + fl.z * 0.35;
  vec3 col = INK + acc;
  // Soft clip: overlapping foci blend instead of blowing out.
  col = col / (1.0 + max(0.0, dot(acc, vec3(0.33)) - 0.55) * 0.9);
  // The preview panel.
  vec2 pc = P.xy + P.zw * 0.5;
  vec2 pd = abs(p - pc) - P.zw * 0.5 + PR;
  float sd = length(max(pd, 0.0)) + min(max(pd.x, pd.y), 0.0) - PR;
  float panel = 1.0 - smoothstep(-0.7, 0.7, sd);
  col = mix(col, INK, panel * 0.84);
  col += grain(0.05);
  o = vec4(col, 1.0);
}`

/** Vertex shader of the GPGPU passes: the triangle's uv, as texture coordinates. */
export const SIM_VERTEX = /* glsl */ `#version 300 es
in vec2 position;
in vec2 uv;
out vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = vec4(position, 0.0, 1.0);
}`

/**
 * The particle simulation, one texel per particle: xy position, zw velocity, in the stage's CSS
 * px. `tInit` holds each particle's constants: position along the waveform, thickness offset,
 * tone, seed. Each step (semi-implicit Euler, `DT` clamped by the caller): a spring to the home
 * on the waveform, and near the pointer (`PTR`: x, y, vx, vy; weight `PW`) a push, a swirl, the
 * pointer's wake and divergence-free curl-noise turbulence. `K`: radius, push, swirl, stiffness;
 * `K2`: damping, wake, turbulence. `RESET` puts every particle at home, motionless.
 */
export const SIM = /* glsl */ `#version 300 es
precision highp float;
in vec2 vUv;
out vec4 o;
uniform sampler2D tMap;
uniform sampler2D tInit;
uniform vec4 B;
uniform float T;
uniform float DT;
uniform vec4 PTR;
uniform float PW;
uniform vec4 K;
uniform vec3 K2;
uniform float RESET;
${SNOISE}
// The decorative waveform of the hero (the same partials as the prerendered SVG), -1..1.
float wave(float x, float t) {
  float env = exp(-2.2 * x) * (1.0 - exp(-40.0 * x));
  float breath = 0.86 + 0.14 * sin(t * 0.9);
  return env * breath * (sin(x * 80.0 + t * 1.1) * 0.7 + sin(x * 211.0 - t * 2.3) * 0.25 + sin(x * 23.0 + t * 0.6) * 0.3);
}
vec2 curl(vec2 p) {
  const float e = 0.05;
  float dy = snoise(p + vec2(0.0, e)) - snoise(p - vec2(0.0, e));
  float dx = snoise(p + vec2(e, 0.0)) - snoise(p - vec2(e, 0.0));
  return vec2(dy, -dx) / (2.0 * e);
}
void main() {
  vec4 st = texture(tMap, vUv);
  vec4 init = texture(tInit, vUv);
  vec2 home = vec2(B.x + init.x * B.z, B.y + B.w * 0.5 - wave(init.x, T) * B.w * 0.39 + init.y);
  if (RESET > 0.5) {
    o = vec4(home, 0.0, 0.0);
    return;
  }
  vec2 pos = st.xy;
  vec2 vel = st.zw;
  vec2 acc = (home - pos) * K.w;
  vec2 d = pos - PTR.xy;
  float r = length(d) + 1e-3;
  float f = max(0.0, 1.0 - r / K.x);
  float k = f * f * PW;
  if (k > 0.0) {
    vec2 dir = d / r;
    acc += dir * k * K.y + vec2(-dir.y, dir.x) * k * K.z + PTR.zw * k * K2.y;
    acc += curl(pos * 0.011 + init.w * 3.0 + T * 0.4) * k * K2.z;
  }
  vel = (vel + acc * DT) * exp(-K2.x * DT);
  pos += vel * DT;
  o = vec4(pos, vel);
}`

/** The waveform's particles: one point per texel, brighter and tinted while it flies. */
export const PARTICLE_VERTEX = /* glsl */ `#version 300 es
in vec2 coords;
uniform sampler2D tMap;
uniform sampler2D tInit;
uniform vec2 S;
uniform float DP;
uniform float TS;
out float vt;
out float vs;
void main() {
  vec2 uv = coords + 0.5 / TS;
  vec4 st = texture(tMap, uv);
  vec4 init = texture(tInit, uv);
  vec2 c = st.xy / S * 2.0 - 1.0;
  gl_Position = vec4(c.x, -c.y, 0.0, 1.0);
  vs = clamp(length(st.zw) / 900.0, 0.0, 1.0);
  gl_PointSize = (1.3 + vs * 1.4) * DP;
  vt = init.z;
}`

export const PARTICLE_FRAGMENT = /* glsl */ `#version 300 es
precision mediump float;
in float vt;
in float vs;
out vec4 o;
const vec3 PAPER = vec3(236.0, 233.0, 230.0) / 255.0;
const vec3 LILAC = vec3(156.0, 146.0, 255.0) / 255.0;
void main() {
  vec2 d = gl_PointCoord - 0.5;
  float a = smoothstep(0.5, 0.15, length(d)) * (vt * 0.75 + vs * 0.45);
  vec3 c = mix(PAPER, LILAC, vs * 0.85);
  o = vec4(c * a, a);
}`
