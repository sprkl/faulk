import { PRELUDE } from './common'

/**
 * DRY KICK: a tight sphere that gets hit. `H` is the time since the last hit, `C` the centre and
 * `R` the radius in CSS px, `PV` the pointer's push on the body (px). On a hit the body punches
 * out and rings with a pitch that drops, its membrane shows travelling rings, a shock ring leaves
 * it (a second one at 62 %, the overtone). `BG` is 1 for an opaque ink stage, 0 for a transparent
 * canvas (the Atelier's example, over its own dark card).
 */
export const KICK = /* glsl */ `${PRELUDE}
uniform vec2 C;
uniform float R;
uniform float H;
uniform vec2 PV;
uniform float BG;
const vec3 K0 = vec3(255.0, 178.0, 122.0) / 255.0;
const vec3 K1 = vec3(255.0, 106.0, 61.0) / 255.0;
const vec3 K2 = vec3(194.0, 24.0, 91.0) / 255.0;
const vec3 K3 = vec3(42.0, 14.0, 92.0) / 255.0;

vec3 ramp(float t) {
  vec3 c = mix(K0, K1, smoothstep(0.0, 0.30, t));
  c = mix(c, K2, smoothstep(0.30, 0.62, t));
  return mix(c, K3, smoothstep(0.62, 1.0, t));
}

void main() {
  vec2 p = v * S;
  float tau = max(H, 0.0);
  float punch = exp(-tau * 16.0);
  // The pitch envelope of a kick: fast at the hit, settling low.
  float phase = 6.2832 * (2.4 * tau + 9.0 * (1.0 - exp(-tau * 6.0)) / 6.0);
  float body = exp(-tau * 4.5) * sin(phase);
  vec2 c = C + PV;
  float r = R * (1.0 + 0.13 * punch + 0.04 * body);
  vec2 d = p - c;
  // The pointer's push also squashes the body a little along its direction.
  float pl = length(PV);
  if (pl > 0.5) {
    vec2 u = PV / pl;
    d += u * dot(d, u) * (-0.08 * min(1.0, pl / 12.0));
  }
  float dist = length(d);
  float edge = 1.0 - smoothstep(r - 0.7 / DP, r + 0.7 / DP, dist);
  vec2 light = c + vec2(-0.08, -0.10) * R;
  float t = length(p - light) / (r * 1.06);
  vec3 col = ramp(t);
  // Membrane rings travelling outward after a hit.
  float ring = sin(dist / r * 20.0 - tau * 26.0) * exp(-tau * 3.2) * (1.0 - dist / r);
  col += ring * 0.09;
  col += punch * 0.32 * (1.0 - t * 0.6);
  col += grain(0.07);
  // Shock rings leaving the body: the hit, and its 62 % overtone a little later.
  float shock = 0.0;
  for (int k = 0; k < 2; k++) {
    float tk = tau - float(k) * 0.11;
    if (tk <= 0.0) continue;
    float rr = R * (1.02 + tk * 3.4);
    float w = R * (0.05 + tk * 0.25);
    float a = exp(-(dist - rr) * (dist - rr) / (w * w)) * exp(-tk * 5.5) * (k == 0 ? 0.55 : 0.34);
    shock += a * step(r, dist);
  }
  // Rings fade before the canvas edge, so a clipped canvas never shows a hard cut.
  float edgeFade = 1.0 - smoothstep(0.0, 1.0, max(max(0.5 - v.x, v.x - 0.5), max(0.5 - v.y, v.y - 0.5)) * 2.0 - 0.72);
  shock *= edgeFade;
  vec3 bg = INK + grain(0.03);
  vec3 outside = bg + K1 * shock;
  float alpha = max(edge, min(1.0, shock * 1.6));
  vec3 final = mix(outside, col, edge);
  if (BG > 0.5) {
    o = vec4(final, 1.0);
  } else {
    o = vec4(mix(K1 * shock, col, edge) * alpha, alpha);
  }
}`

/**
 * HAZE: fog. Three soft foci (lilac, sky, a touch of paper) in a doubly warped simplex field
 * that drifts slowly; the pointer's flow map stirs it, and the fog keeps swirling after it.
 */
export const HAZE = /* glsl */ `${PRELUDE}
uniform vec4 F[3];
const vec3 H0 = vec3(156.0, 146.0, 255.0) / 255.0;
const vec3 H1 = vec3(59.0, 185.0, 237.0) / 255.0;
const vec3 H2 = vec3(253.0, 252.0, 252.0) / 255.0;

void main() {
  vec2 p = v * S;
  float side = min(S.x, S.y);
  vec3 fl = flow(v, side * 0.45);
  vec2 n = (p + fl.xy * 0.6) / (side * 0.8);
  vec2 w1 = vec2(fbm(n * 0.7 + T * 0.02), fbm(n * 0.7 + vec2(3.1, 7.7) - T * 0.016));
  vec2 q = p + fl.xy + w1 * side * 0.22;
  vec2 w2 = vec2(fbm(q / side * 2.2 + T * 0.05), fbm(q / side * 2.2 + vec2(9.2, 2.4) + T * 0.04));
  q += w2 * side * 0.07;
  vec3 acc = vec3(0.0);
  vec3 C[3] = vec3[3](H0, H1, H2);
  float breath = 1.0 + 0.08 * sin(T * 0.5);
  for (int i = 0; i < 3; i++) {
    vec2 c = F[i].xy * S;
    float d = length(q - c) / (F[i].z * S.x * breath);
    acc += C[i] * exp(-d * d * 2.6) * (i == 2 ? 0.2 : 0.85);
  }
  // Fog density: thin at the stage's edges, like the blurred object it replaces.
  vec2 e = (v - 0.5) * 2.0;
  float mask = 1.0 - smoothstep(0.45, 1.0, length(e * vec2(1.0, 0.9)));
  vec3 col = INK + acc * mask * (1.0 + fl.z * 0.3);
  col = col / (1.0 + max(0.0, dot(acc, vec3(0.33)) - 0.6) * 0.9);
  col += grain(0.035);
  o = vec4(col, 1.0);
}`

/**
 * CRUSH 8: a square of warm colour (conic gradient) degraded digitally. Everything is stepped:
 * sample-and-hold blocks (`block` px), colour depth with ordered dither, rows that slip, a stepped
 * tilt. `CR` is the crush amount (0..1) the pointer raises nearby, `TS` the stepped time.
 */
export const CRUSH = /* glsl */ `${PRELUDE}
uniform vec2 C;
uniform float R;
uniform float CR;
uniform float TS;
const vec3 Q0 = vec3(238.0, 53.0, 51.0) / 255.0;
const vec3 Q1 = vec3(239.0, 204.0, 54.0) / 255.0;
const vec3 Q2 = vec3(42.0, 14.0, 92.0) / 255.0;

vec3 conic(float a) {
  // Stops 0 red, .25 yellow, .5 red, .75 deep violet, 1 red.
  float t = fract(a / 6.2832);
  vec3 c = mix(Q0, Q1, smoothstep(0.0, 0.25, t));
  c = mix(c, Q0, smoothstep(0.25, 0.5, t));
  c = mix(c, Q2, smoothstep(0.5, 0.75, t));
  return mix(c, Q0, smoothstep(0.75, 1.0, t));
}

void main() {
  vec2 p = v * S;
  float cr = clamp(CR + (h21(vec2(TS, 3.1)) - 0.5) * 0.25, 0.0, 1.0);
  float block = mix(2.5, 11.0, cr);
  // Sample and hold: the image is only known at block centres.
  vec2 pq = (floor(p / block) + 0.5) * block;
  // Rows slip sideways now and then, more when crushed.
  float row = floor(pq.y / (block * 3.0));
  float slip = step(0.8, h21(vec2(row * 1.7, TS + 1.0))) * (h21(vec2(row, TS)) - 0.5) * 18.0 * (0.3 + cr);
  pq.x += slip;
  vec2 d = pq - C;
  // A small stepped tilt that never settles, never turns.
  float ang = (h21(vec2(floor(TS * 0.8), 9.0)) - 0.5) * 0.16;
  d = spin(d, vec2(0.0), ang);
  float rad = R * 0.157;
  vec2 q = abs(d) - (R - rad);
  float sd = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - rad;
  float inside = step(sd, 0.0);
  float a = atan(d.y, d.x) + 3.665 + ang;
  vec3 col = conic(a);
  // Colour depth: 6 levels clean, 3 when crushed, with ordered dither.
  float levels = mix(6.0, 3.0, cr);
  col = floor(col * levels + bayer(pq / block) * 0.9 + 0.5) / levels;
  col += (h21(floor(p / (block * 0.5)) + floor(TS * 12.0)) - 0.5) * 0.09;
  vec3 bg = INK + grain(0.03);
  o = vec4(mix(bg, col, inside), 1.0);
}`

/**
 * ECHO: ripples on a dark surface. Each drop (`D`: x, y in stage fractions, start time,
 * amplitude) sends waves outward and repeats itself at 62 % after each delay, like a feedback
 * delay; the waves of different drops interfere. The surface is lit from above-left so the
 * ripples read as relief, in the object's teal.
 */
export const ECHO = /* glsl */ `${PRELUDE}
uniform vec2 C;
uniform float R;
uniform vec4 D[12];
const vec3 TEAL = vec3(0.0, 197.0, 183.0) / 255.0;
const float FEEDBACK = 0.62;
const float DELAY = 0.42;

float ripple(vec2 p, vec4 drop, float side) {
  float h = 0.0;
  vec2 c = drop.xy * S;
  float dist = length(p - c) / side;
  for (int k = 0; k < 3; k++) {
    float age = T - drop.z - float(k) * DELAY;
    if (age <= 0.0) continue;
    float front = age * 0.42;
    float env = exp(-age * 0.9) * pow(FEEDBACK, float(k));
    // Waves behind the front only, strongest just behind it.
    float behind = smoothstep(front + 0.02, front - 0.06, dist);
    h += drop.w * env * behind * sin(dist * 52.0 - age * 9.0) * exp(-dist * 2.2);
  }
  return h;
}

float height(vec2 p, float side) {
  float d0 = length(p - C);
  float h = 0.3 * sin(d0 / side * 46.0 - T * 0.6) * exp(-d0 / side * 1.2);
  for (int i = 0; i < 12; i++) h += ripple(p, D[i], side);
  return h;
}

void main() {
  vec2 p = v * S;
  float side = min(S.x, S.y);
  float h = height(p, side);
  float eps = 1.5;
  float hx = height(p + vec2(eps, 0.0), side) - h;
  float hy = height(p + vec2(0.0, eps), side) - h;
  vec3 n = normalize(vec3(-hx * 9.0, -hy * 9.0, 1.0));
  vec3 L = normalize(vec3(-0.5, -0.6, 0.7));
  float diff = max(dot(n, L), 0.0);
  float spec = pow(max(dot(reflect(-L, n), vec3(0.0, 0.0, 1.0)), 0.0), 18.0);
  float lvl = clamp(h * 0.9 + 0.5, 0.0, 1.0);
  vec3 col = mix(INK, TEAL, pow(lvl, 1.6) * 0.95);
  col += TEAL * diff * 0.18 + vec3(0.8, 1.0, 0.98) * spec * 0.35;
  float d0 = length(p - C) / R;
  float mask = 1.0 - smoothstep(0.5, 1.02, d0);
  vec3 bg = INK + grain(0.03);
  col = mix(bg, col + grain(0.05), mask);
  o = vec4(col, 1.0);
}`
