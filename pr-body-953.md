## Pourquoi

Un morceau qui change de tonalité en cours de route (hausse d'un ton, visite du ton relatif) ne pouvait pas le dire à Faulk : le projet n'avait qu'une gamme (#695), et depuis #948 le piano roll nomme les touches noires d'après elle. Ici, la tonalité se pose à une mesure, et tout ce qui suit la gamme du projet suit la tonalité en vigueur à cet endroit.

## Ce qui change

- **Modèle et format** : `key_changes` dans `project.json` (liste triée par tick, un changement par tick, jamais au tick 0 qui reste la tonalité de départ `scale`, au plus 256), absent quand elle est vide. **Format de dossier 9** (provisoire : 8 est celui de #952/#960, le lead réattribue dans l’ordre de fusion) ; la migration de 8 à 9 ne change que le numéro, comme les précédentes.
- **Commandes** (domaine `arrangement`) : `arrangement.key_change.set` (pose ou remplace), `.move` (refusée vers une mesure occupée, pour que l'annulation soit un simple déplacement inverse), `.remove`, `.list`. Toutes annulables, avec schéma et description pour les agents ; `project.get` rend `key_change_count`. Le tick doit être un début de mesure après la première, vérifié à la commande.
- **Moteur** : `ControlMsg::SetKeyChanges` (tag 5, la liste entière), table de taille fixe lue par recherche dichotomique, échangée par `apply` (l'ancienne devient `Garbage::KeyChanges`). Chaque bloc reçoit la tonalité à la tête de lecture et jusqu'à huit changements dans le bloc (`Transport::scale_at(frame)`) : Stack, Lattice et Scatter jouent chaque note dans la tonalité de son propre tick, en lecture comme à l'export, quelle que soit la taille du bloc, jusqu’à huit changements par bloc (y compris une boucle qui retombe dans une autre tonalité). Aucune allocation sur le chemin audio.
- **Interface** : les changements apparaissent dans la règle de l'arrangement (clic, Alt+flèches d'une mesure, glisser, Suppr, menu contextuel, et « Ajouter un changement de tonalité ici » dans le menu « Plus ») ; le bouton de gamme du transport montre la tonalité à la tête de lecture, et son panneau modifie la tonalité sur laquelle il a été ouvert (le changement en vigueur, ou la tonalité de départ avant le premier), propose d'ajouter ou de retirer un changement ; le piano roll nomme et surligne d'après la tonalité en vigueur au début du placement ouvert. Textes en anglais et en français.
- **Protocole** : `PROTOCOL_VERSION` 61, **provisoire** (59 est #949, 60 est #952) ; nouvelle sorte de message de contrôle : tag 5 (`SetKeyChanges`) ; ni retour moteur ni sorte de device nouveaux.

## Vérification

- `node scripts/agent/verify.mjs` (mode complet) : fmt, clippy, `cargo test --workspace` (dont les rendus natif et WASM en parité), builds WASM du moteur et du projet, scripts, format, `gen:check`, typecheck, lint, vitest, suite e2e complète de 102 specs.
- Tests ajoutés : `crates/project/tests/it/key_changes.rs` (format, migration 7 → 8, cache, pack, refus), `crates/commands/tests/it/key_change.rs` (+ exemples du contrat et du test commandes ↔ outils MCP), `crates/engine/tests/it/keymap.rs` (chaque tick de chaque bloc voit la tonalité de son propre tick pour des blocs de 1 à 2048 trames, sur un changement, une boucle qui retombe dans une autre tonalité et un saut, sous le détecteur d'allocation), `crates/test-support/tests/it/key_change.rs` (rendu Lattice natif et WASM : la première note de la mesure qui commence la nouvelle tonalité est dans la nouvelle tonalité), `growth.rs` et `msg.rs` (échange et ramasse-miettes, codec), `web/tests/e2e/key-changes.spec.ts` (poser, déplacer, supprimer dans la règle, bouton de gamme qui suit la tête de lecture, noms du piano roll dans les deux tonalités, EN et FR).
- Vérification à la main dans le navigateur (Playwright) : capture de la règle avec un marqueur et un changement de tonalité sur la même mesure.

## Revues

Judge Dredd, revue du plan puis de la livraison : les constats et ce qu'ils sont devenus sont dans le commentaire de l'issue.

## Hors périmètre

- Insérer ou supprimer du temps (`arrangement.insert_time`, `delete_time`) ne déplace pas les changements de tonalité, contrairement aux marqueurs (`restore_time` porte les marqueurs explicitement et demanderait un champ de plus). À décider.
- Pas de « transposer ce qui suit » (étiquette seulement, comme décidé au tri).
- Les commandes `set`, `move` et `remove` ne sont pas dans la palette (on les place par mesure, dans la règle et le panneau de gamme).
- Plus de huit changements de tonalité dans un seul bloc (boucle de quelques millisecondes à cheval sur des tonalités) : les suivants sont laissés de côté.

## Commit message

```
feat(project,commands,engine,web): key changes placed at a bar

The project may hold key changes after its starting key: from the bar that
starts at each one's tick, the key and scale change. Folder format 9 (field
`key_changes`, absent when empty; the migration from 8 changes only the
number). Commands `arrangement.key_change.set`, `.move`, `.remove`, `.list`,
undoable; `project.get` answers `key_change_count`.

The engine takes the whole list (`ControlMsg::SetKeyChanges`, tag 5, protocol
61 provisional) in a fixed-size table swapped in by `apply` (the old one is
garbage), and gives each block the key at the playhead and up to eight
switches inside it, so Stack, Lattice and Scatter play every note in the key
of its own tick, in playback and export, whatever the block size (up to eight key switches per block).

The arrangement ruler shows each key change (click, Alt+arrows, drag by bars,
Delete, menu); the transport's key button follows the playhead and its panel
edits the key it was opened on and adds or removes a key change; the piano
roll names and shades the notes from the key in force where the placement it
was opened from starts. English and French.

Closes #953

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_016MUT29q8jE1VkEeht8DfgP
```

🤖 Generated with [Claude Code](https://claude.com/claude-code)

https://claude.ai/code/session_016MUT29q8jE1VkEeht8DfgP
