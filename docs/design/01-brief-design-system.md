---
type: Design
title: "Brief du design system"
description: "Pointer to the design system brief and to the DS-0.1 canvas that is the design reference, and their key decisions."
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

## Design system DS-0.1

Le canvas [Faulk DS-0.1](https://claude.ai/artifact/FrH1dzkY5Xgyn7JdgsU4eQ) fait foi depuis le 2026-10-13 (décision du fondateur). Ses tokens sont copiés dans `web/src/lib/styles/tokens.css` : accent ultraviolet #6633F4, neutres chauds, application en thème sombre, Archivo (titres), Geist (interface), Geist Mono (valeurs), base d'espacement 4 px. Chaque état (lecture, enregistrement, solo, mute, Boost) a un glyphe en plus de sa couleur.

## Questions ouvertes

Piste de logo, vue unifiée clips et timeline à prototyper.

## Contexte pour les agents (2026-11-01)

Trois fichiers à la racine du dépôt traduisent ce brief pour les agents de design (impeccable) : [`PRODUCT.md`](../../PRODUCT.md) (cible, positionnement, principes, accessibilité), [`DESIGN.md`](../../DESIGN.md) (le système visuel « The Swiss Instrument », ses tokens et ses règles, avec les écarts actuels entre le canvas DS-0.1 et le code) et `.impeccable/design.json` (sa version lisible par l'outil). Le canvas DS-0.1 reste la référence visuelle : `DESIGN.md` en est la traduction écrite, et en cas d'écart le canvas prime jusqu'à ce que `DESIGN.md` soit mis à jour. Décisions du fondateur du 2026-11-01 : l'app vise les producteurs confirmés sur grands écrans (1440 px et plus, pas de tablette), densité confort par défaut, et les actions d'un agent sont visibles, attribuées et annulables.
