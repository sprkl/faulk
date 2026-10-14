---
type: Spike
title: "S1 — Moteur hybride web et natif"
description: "First spike: prove that one Rust engine runs both in an AudioWorklet and in a native helper with ASIO, that one command layer drives it from the UI, an MCP agent and an ACP agent, and measure the latency of each mode."
status: draft
---

# S1 — Moteur hybride web et natif

Prouver qu'un seul moteur Rust tourne dans le navigateur et dans un helper natif, qu'une seule couche de commandes le pilote depuis l'interface comme depuis un agent, et mesurer la latence de chaque mode.

## Étapes

1. Workspace Rust (`engine`, `commands`, `protocol`, `engine-wasm`, `helper`) et app web minimale.
2. Une première couche de commandes : `transport.play`, `transport.stop`, `track.create`, `note.add`, `param.set`, chacune avec son schéma, sa description et son annulation. L'interface n'utilise que ces commandes.
3. Mode Web : le moteur en WASM dans un `AudioWorklet` joue un synthé simple, déclenché par Web MIDI ou le clavier de l'ordinateur.
4. Mode Boost : le même moteur dans le helper, sortie ASIO ou WASAPI via `cpal` ; la page détecte le helper, s'appaire avec lui par un jeton et bascule.
5. Un module Faust chargé dans les deux modes.
6. Un plugin CLAP tiers chargé dans le helper, fenêtre ouverte.
7. Serveur MCP dans le helper : les outils sont générés depuis la couche de commandes. Claude Code s'y connecte, crée une piste et y pose des notes, et le résultat s'affiche et s'entend dans la page ouverte.
8. Session ACP : depuis la page, le helper lance Claude Code en agent ACP, relaie la conversation vers un panneau de la page, et fournit à l'agent le serveur MCP de Faulk. Une demande en langage naturel (« ajoute une piste de basse avec quatre notes ») modifie le projet.
9. Mesure de la latence dans les deux modes.

## Critères de réussite

- Un seul code moteur, sans code spécifique à une cible hors des couches d'entrée-sortie.
- Le même rendu hors ligne donne le même signal en natif et en WASM, à une tolérance fixée.
- Chaque commande de l'étape 2 est appelable depuis l'interface, en MCP et par l'agent ACP, sans code dupliqué ; un test vérifie que toute commande est exposée en outil MCP.
- Les actions de l'agent apparaissent dans l'historique avec leur auteur et s'annulent en une fois.
- Une requête vers le helper sans jeton d'appairage, ou depuis une autre origine que Faulk, est refusée.
- Latences mesurées et notées ici, avec la méthode.

## Hors du spike

WebMCP (encore expérimental) et les agents ACP distants (transport en cours de spécification).

## Prérequis sur la machine

Rust (`rustup`, cible `wasm32-unknown-unknown`), sous Windows les Visual Studio Build Tools (charge de travail C++) pour le helper, Node 24 et pnpm, le compilateur Faust, Claude Code.
