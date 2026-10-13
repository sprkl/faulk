---
type: Architecture
title: "Architecture technique"
description: "One Rust engine compiled to WebAssembly for the browser and to native code for the helper, driven by one web UI; modules in Faust or WASM; third-party plugins in the helper only."
status: draft
---

# Architecture technique

Un seul moteur audio en Rust, compilé pour deux cibles, piloté par une seule interface web. On déplace le moteur, jamais le son.

```
              Interface web (identique dans les deux modes)
                       │ commandes, retours visuels
           ┌───────────┴────────────┐
           ▼                        ▼
   MODE WEB (défaut)          MODE BOOST (helper installé)
   moteur → WASM              moteur natif
   dans un AudioWorklet       • ASIO / WASAPI / CoreAudio via cpal
   • modules Faust/WASM       • modules Faust/WASM (les mêmes)
                              • plugins VST3 / CLAP tiers
                              • MIDI direct (midir)
```

## Composants

| Composant | Rôle | Technologies envisagées |
| --- | --- | --- |
| `crates/engine` | Graphe audio, transport, horloge, DSP. Pur : aucune entrée-sortie, aucune allocation dans le chemin audio. | Rust |
| `crates/engine-wasm` | Le moteur dans un `AudioWorklet`. Mémoire partagée avec l'interface via `SharedArrayBuffer`. | Rust, `wasm-bindgen` |
| `crates/helper` | Le moteur natif, l'audio basse latence, le MIDI, l'hébergement de plugins tiers, un serveur WebSocket local. | Rust, `cpal`, `midir`, `clack` (CLAP), bindings VST3 |
| `crates/protocol` | Les messages interface ↔ moteur, définis une fois, types TypeScript générés. | Rust, `serde` |
| `web/` | L'interface : arrangement, clips, mixer, Atelier. | TypeScript, Vite ; framework à choisir |
| `modules/` | Les modules de Faulk. | Faust d'abord (compilation à la volée pour le rechargement à chaud), Rust → WASM ensuite |

## Principes

1. **Un seul moteur.** Toute fonctionnalité du moteur doit compiler et se comporter pareil en WASM et en natif, testée par un rendu comparé.
2. **Le son ne traverse jamais le lien interface ↔ helper.** Seuls passent les commandes (quelques octets) et les retours visuels (vumètres, formes d'onde). La latence du lien ne s'entend pas.
3. **Un helper bête.** Pas d'interface, pas de logique produit : il exécute le moteur. Les fenêtres des plugins tiers s'y ouvrent comme des fenêtres natives.
4. **Des projets portables.** Un projet utilisant un plugin tiers reste ouvrable sans helper : la piste concernée est gelée (rendue en audio) et rejouée telle quelle.
5. **Modules isolés.** Un module en WASM s'exécute isolé : un module bogué ne fait pas planter la session.

## Points à valider (spike S1)

- Latence réelle du mode Web avec `renderSizeHint` (Chrome 153+) sur la machine du fondateur.
- Un seul code moteur réellement partagé entre les deux cibles, sans divergence.
- Détection du helper depuis la page (`ws://localhost`, autorisation d'accès au réseau local de Chrome) et bascule à chaud.
- Compilation Faust à la volée dans les deux cibles.
