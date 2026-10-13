---
type: Constraints
title: "Contraintes transverses"
description: "Real-time audio, latency targets, browser limits, security of the local bridge, licences (VST3, ASIO, GPL), and the legal stance on recreating existing plugins."
status: draft
---

# Contraintes transverses

## Temps réel

Le chemin audio n'alloue pas, ne verrouille pas, ne bloque pas et ne panique pas. Règles détaillées dans `CLAUDE.md` à la racine du dépôt.

## Latence

| Mode | Cible | Limite connue |
| --- | --- | --- |
| Web | La plus basse possible, mesurée | Pas d'ASIO ni de mode exclusif dans le navigateur ; mixeur du système ; `renderSizeHint` (Chrome 153+) permet des blocs de 64 échantillons |
| Boost | Sous 10 ms en sortie | Dépend du pilote et de la carte son |

Une latence n'est jamais annoncée sans mesure.

## Navigateurs

- Web MIDI : Chrome, Edge, Firefox ; pas Safari.
- `SharedArrayBuffer` exige l'isolation cross-origin (en-têtes COOP et COEP).
- Le helper sur `localhost` déclenche une demande d'autorisation d'accès au réseau local dans Chrome : l'interface doit l'expliquer.

## Sécurité du pont local

N'importe quel site ouvert dans le navigateur peut tenter de joindre un serveur sur `localhost`. Le helper :

- n'écoute que sur `127.0.0.1` ;
- exige un jeton d'appairage, créé lors d'une confirmation de l'utilisateur, sur chaque connexion de la page et du serveur MCP en HTTP ;
- vérifie l'en-tête `Origin` (seules les origines de Faulk sont acceptées) et l'en-tête `Host` (contre le DNS rebinding) ;
- ne lance que les agents ACP que l'utilisateur a configurés, jamais une commande reçue de la page.

Les commandes destructrices demandent une confirmation de l'utilisateur, quel que soit l'appelant.

## Licences

- **VST3** : SDK sous licence MIT depuis la version 3.8 (octobre 2025).
- **ASIO** : SDK sous double licence, propriétaire ou GPLv3 (fin 2025). Utiliser la GPLv3 rend le helper GPLv3 : décision à prendre avant d'intégrer ASIO.
- **Plugins open source portés** : beaucoup sont GPL (Surge XT, Dexed) ; Airwindows est MIT. Un module dérivé de code GPL est GPL.

## Recréer des plugins existants

- **Interdit** : décompiler un plugin commercial pour en reproduire le code. Les licences l'interdisent et la reproduction serait une contrefaçon.
- **Autorisé** : la recréation en boîte noire par mesure (signaux de test, comparaison des réponses), qui reproduit un comportement sans copier de code. Ne jamais reprendre le nom, la marque ou l'apparence du plugin d'origine.
