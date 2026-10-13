---
type: Design
title: "Brief du design system"
description: "Pointer to the design system brief given to Claude Design, and its key decisions: references, foundations, the two surfaces and the DAW interface principles."
status: draft
---

# Brief du design system

Le brief complet est un document partagé : [Faulk — Brief design system](https://claude.ai/code/artifact/b07bd1ac-b7c7-43e6-b127-8eb226f561a0). Ce fichier en garde l'essentiel pour le dépôt.

## Références

- [ElevenLabs](https://elevenlabs.io/) pour les textures : volumes en dégradé granuleux, fond blanc cassé, typographie fine.
- [Akuto Studio](https://akutostudio.com/) pour le site : typographie géante en capitales, une couleur d'accent en aplat, esprit matériel (tableaux de spécifications).

## Décisions clés

- Un seul jeu de tokens, deux thèmes : clair pour le site, sombre par défaut pour l'application.
- Chaque module a son « objet sonore » : une texture granuleuse générée à partir de ses paramètres.
- Une seule couleur d'accent, propre à Faulk.
- L'application répond aux critiques courantes des DAW : contraste et taille de texte, vue unifiée clips et timeline à explorer, palette de commandes, plugins ancrés dans la chaîne d'effets, routage visuel, historique de versions, dévoilement progressif.
- L'état du moteur (mode Web ou Boost, latence, charge processeur) est toujours visible.

## Questions ouvertes

Couleur d'accent, thème par défaut de l'application, licences de polices, piste de logo, vue unifiée à prototyper.
