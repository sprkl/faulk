/**
 * GLSL ES 3.00 shared by every object. Fragment shaders receive `v` (0..1 over the stage, y
 * down like CSS), the stage size `S` in CSS px, the time `T` in seconds, the pointer `M` in CSS
 * px with its weight `MI` (0 absent, 1 driving) and `DP`, device px per CSS px.
 *
 * Grain: every object dithers its colours with a hash noise re-seeded 12 times per second, like
 * film grain, far too small to read as a flash (WCAG 2.3.1 counts luminance changes of 10 % over
 * large areas; this is 3 to 7 % per pixel, uncorrelated).
 */

/** OGL's `Triangle` gives `position` (-1..1) and `uv` (0..1, y up); `v` is y down. */
export const VERTEX = /* glsl */ `#version 300 es
in vec2 position;
in vec2 uv;
out vec2 v;
void main() {
  v = vec2(uv.x, 1.0 - uv.y);
  gl_Position = vec4(position, 0.0, 1.0);
}`

/**
 * 2D simplex noise: Ian McEwan and Stefan Gustavson, Ashima Arts, MIT licence
 * (https://github.com/stegu/webgl-noise). Returns about -1..1.
 */
export const SNOISE = /* glsl */ `
vec3 mod289(vec3 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec2 mod289(vec2 x) { return x - floor(x * (1.0 / 289.0)) * 289.0; }
vec3 permute(vec3 x) { return mod289(((x * 34.0) + 1.0) * x); }
float snoise(vec2 vv) {
  const vec4 C = vec4(0.211324865405187, 0.366025403784439, -0.577350269189626, 0.024390243902439);
  vec2 i = floor(vv + dot(vv, C.yy));
  vec2 x0 = vv - i + dot(i, C.xx);
  vec2 i1 = (x0.x > x0.y) ? vec2(1.0, 0.0) : vec2(0.0, 1.0);
  vec4 x12 = x0.xyxy + C.xxzz;
  x12.xy -= i1;
  i = mod289(i);
  vec3 p = permute(permute(i.y + vec3(0.0, i1.y, 1.0)) + i.x + vec3(0.0, i1.x, 1.0));
  vec3 m = max(0.5 - vec3(dot(x0, x0), dot(x12.xy, x12.xy), dot(x12.zw, x12.zw)), 0.0);
  m = m * m;
  m = m * m;
  vec3 x = 2.0 * fract(p * C.www) - 1.0;
  vec3 h = abs(x) - 0.5;
  vec3 ox = floor(x + 0.5);
  vec3 a0 = x - ox;
  m *= 1.79284291400159 - 0.85373472095314 * (a0 * a0 + h * h);
  vec3 g;
  g.x = a0.x * x0.x + h.x * x0.y;
  g.yz = a0.yz * x12.xz + h.yz * x12.yw;
  return 130.0 * dot(m, g);
}
`

export const PRELUDE = /* glsl */ `#version 300 es
precision highp float;
in vec2 v;
out vec4 o;
uniform vec2 S;
uniform float T;
uniform vec2 M;
uniform float MI;
uniform float DP;
const vec3 INK = vec3(19.0, 18.0, 17.0) / 255.0;
${SNOISE}
// Fractal noise, three octaves rotated against each other, about -1..1.
float fbm(vec2 p) {
  const mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
  float s = 0.0, a = 0.5;
  for (int k = 0; k < 3; k++) {
    s += a * snoise(p);
    p = rot * p * 2.0 + 7.1;
    a *= 0.5;
  }
  return s;
}
float h21(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}
// Film grain, re-seeded 12 times per second.
float grain(float amp) {
  return (h21(floor(gl_FragCoord.xy) + floor(T * 12.0) * 7.0) - 0.5) * amp;
}
// 4x4 ordered dither, -0.5..0.5.
float bayer(vec2 p) {
  ivec2 i = ivec2(mod(p, 4.0));
  int n = (i.x ^ i.y) & 1 | ((i.x >> 1 ^ i.y >> 1) & 1) << 1 | (i.x & 1) << 2 | (i.x >> 1 & 1) << 3;
  return float(n) / 16.0 - 0.5;
}
// Rotates p around c by angle a.
vec2 spin(vec2 p, vec2 c, float a) {
  vec2 d = p - c;
  float cs = cos(a), sn = sin(a);
  return c + vec2(d.x * cs - d.y * sn, d.x * sn + d.y * cs);
}
// The pointer's flow field (OGL Flowmap: xy velocity, z strength), as a displacement in CSS px.
uniform sampler2D tFlow;
uniform float FLOW;
vec3 flow(vec2 uv, float scale) {
  vec3 f = texture(tFlow, vec2(uv.x, 1.0 - uv.y)).rgb * FLOW;
  return vec3(f.x * scale, -f.y * scale, f.z);
}
`
