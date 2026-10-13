---
type: Spike
title: "S1 — Moteur hybride web et natif"
description: "First spike: prove that one Rust engine runs both in an AudioWorklet and in a native helper with ASIO, driven by the same web page, and measure the latency of each."
status: draft
---

# S1 — Moteur hybride web et natif

Prouver qu'un seul moteur Rust tourne dans le navigateur et dans un helper natif, piloté par la même page, et mesurer la latence de chaque mode.

## Étapes

1. Workspace Rust (`engine`, `engine-wasm`, `helper`, `protocol`) et app web minimale.
2. Mode Web : le moteur en WASM dans un `AudioWorklet` joue un synthé simple, déclenché par Web MIDI ou le clavier de l'ordinateur.
3. Mode Boost : le même moteur dans le helper, sortie ASIO ou WASAPI via `cpal` ; la page détecte le helper et bascule.
4. Un module Faust chargé dans les deux modes.
5. Un plugin CLAP tiers chargé dans le helper, fenêtre ouverte.
6. Mesure de la latence dans les deux modes.

## Critères de réussite

- Un seul code moteur, sans code spécifique à une cible hors des couches d'entrée-sortie.
- Le même rendu hors ligne donne le même signal en natif et en WASM, à une tolérance fixée.
- Latences mesurées et notées ici, avec la méthode.

## Prérequis sur la machine

Rust (`rustup`, cible `wasm32-unknown-unknown`), `wasm-pack` ou `wasm-bindgen-cli`, Node et pnpm, le compilateur Faust.
