---
type: Product
title: "Vision produit"
description: "Faulk in one sentence, target users, the Web, Boost and Atelier modes, and the open product decisions."
status: draft
---

# Vision produit

Faulk est un DAW (station de musique assistée par ordinateur) ouvert et portable : il s'ouvre dans le navigateur, et on y fabrique ses propres instruments et effets.

Les DAW actuels sont d'excellents outils, mais fermés et liés à une machine. Faulk mise sur le web pour l'ouverture et la portabilité, et comble l'écart de performance avec un helper natif optionnel, le temps que les navigateurs rattrapent leur retard sur l'audio. La documentation ne cite aucun autre DAW : elle décrit ce que fait Faulk, pas ce à quoi il ressemble.

## Cible

- D'abord le fondateur : un outil à soi pour produire sa musique.
- Ensuite des producteurs et beatmakers qui veulent créer leurs propres sons, et une communauté qui partage ses modules.

## Trois modes, une seule interface

| Mode | Ce qui tourne | Ce que l'utilisateur gagne |
| --- | --- | --- |
| Web | Le moteur audio, compilé en WebAssembly, dans le navigateur | Zéro installation ; latence de 20 à 60 ms selon le navigateur et le matériel |
| Boost | Le même moteur, natif, dans un helper installé et piloté par la même interface web | Latence pro (ASIO, sous 10 ms visés), MIDI direct, plugins VST3 et CLAP tiers |
| Atelier | L'éditeur de modules intégré | Créer un instrument ou un effet en le décrivant à une IA, l'entendre en direct, l'ajuster |

Le passage en Boost est un changement d'état visible, pas une autre application.

## Vocabulaire

- **Moteur** : le cœur audio (graphe, transport, DSP), un seul code pour les deux modes.
- **Helper** : le petit programme natif du mode Boost, sans interface propre.
- **Module** : un instrument ou un effet propre à Faulk, écrit en Faust ou en Rust compilé en WASM, qui tourne dans les deux modes.
- **Plugin tiers** : un VST3 ou CLAP de la communauté, disponible seulement en mode Boost.
- **Objet sonore** : la texture visuelle granuleuse propre à chaque module, générée à partir de ses paramètres.

## La bibliothèque de modules

Les modules sont l'actif central : portables entre les deux modes et partageables. Trois sources :

1. Le portage de plugins open source (Airwindows, Surge XT, Dexed…) en respectant leurs licences.
2. La recréation par mesure : on mesure un plugin existant en boîte noire et l'IA écrit un module équivalent jusqu'à un écart mesuré acceptable (`../constraints/01-contraintes.md`).
3. La création originale dans l'Atelier.

## Décisions ouvertes

- Langue de l'interface : français d'abord, anglais d'abord, ou les deux dès le départ.
- Framework de l'interface web.
- Modèle de distribution : outil personnel, open source, produit commercial.
