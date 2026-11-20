---
type: Spike
title: "Hôte headless — composer et rendre sans navigateur"
description: "Spike of the headless epic (#820): a browserless host, the faulk binary, that owns a project folder, runs the page's project core natively, renders offline and serves the command catalog as MCP tools over stdio; what it proves, what it leaves out, and the founder's decisions it waits on (#821)."
status: draft
---

# Hôte headless — composer et rendre sans navigateur

Spike de l'épic #820, branche `spike/820-headless-proto`. Il vérifie qu'un agent ou un script peut composer et rendre un morceau Faulk **sans page**, depuis un dossier projet, en réutilisant le cœur de projet de la page plutôt qu'en le réécrivant.

Le prototype repose sur les **réponses provisoires** de #821, qui ne valent pas décision du fondateur. La règle cœur « un seul propriétaire du projet à la fois » de `CLAUDE.md` est le texte proposé dans #821, en attente de cette décision.

## Ce que fait le prototype

Le crate `crates/headless` produit le binaire `faulk` :

```
faulk new    <dossier> [--name <nom>]
faulk render <dossier> [-o <fichier.wav>] [--region song|loop] [--rate <hz>] [--format f32|s24|s16]
faulk run    <dossier> <commandes.jsonl | -> [--save] [--keep-going] [--author <nom>]
faulk mcp    <dossier> --stdio
```

- **Cœur partagé.** L'hôte lie nativement le `Core` de `crates/project-wasm` (le même que la page) : exécution des commandes, historique, tours d'agent, amorçage des identifiants, sauvegarde des seuls fichiers modifiés. Il n'écrit aucune logique de commande à lui.
- **Dossier.** L'ouverture construit le cadre de dossier attendu par le cœur, puis lance `project.open` ; la sauvegarde applique le cadre de sauvegarde du cœur par `project::file::StdFs` (fichier temporaire puis renommage).
- **Rendu.** Le job d'export du cœur est rendu par `engine::offline`, le moteur de l'export. Le WAV est identique octet pour octet à l'export natif sur le même état (test `the_headless_export_is_the_native_export_byte_for_byte`). L'export va dans `exports/<nom du projet>.wav` ; un fichier existant est gardé et le nouveau numéroté.
- **Politique (`policy.rs`).** Vérifiée par énumération sur `commands::catalog()` : toute commande `live` est refusée (`headless.unsupported`) sauf `project.save`, `project.export_audio`, `history.begin_group` et `history.end_group` ; `project.new` et `project.open` sont refusées ; `project.list_stored` et `mixer.levels` aussi ; `project.analyze_render` est servie.
- **MCP.** Les outils sont générés depuis le catalogue, moins les commandes refusées, avec les noms, schémas et annotations du helper. Un appel s'exécute en `agent:<clientInfo.name>` ; le dossier est sauvegardé après chaque appel appliqué qui a modifié le projet ; la fin de session termine les tours des agents et sauvegarde.

## Vérification

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p headless` (7 tests unitaires, 14 d'intégration, dont un morceau composé par un agent MCP de bout en bout). La revue de livraison de Judge Dredd a été traitée dans le commit `497d4a80` (frame de dossier limitée aux fichiers du projet comme la page, tailles vérifiées avant lecture, `render -o` atomique, `host.unsaved` quand la sauvegarde échoue après une commande appliquée, route de chaque commande vérifiée sur tout le catalogue).

## Ce qui manque

- Import audio (`clip.import_audio_file`, #827), transport HTTP (#829), verrou `.faulk/lock` (#830, reporté : ne pas ouvrir le même dossier dans la page en même temps).
- Bibliothèque de presets de l'utilisateur et niveaux : l'hôte n'appelle pas `stage_library`, `preset_file` ni `stage_levels`.
- La sauvegarde automatique suit chaque appel, sans le délai de la page.
- L'adaptateur MCP (`tool_name`, `command_json`, `agent_author`, `tool_result`) copie celui du helper au lieu de le partager (#824) ; le `Core` est lié depuis `project-wasm` au lieu d'être extrait (#823).
- Non testés : une sauvegarde interrompue au milieu, un lien symbolique dans `blobs/` qui sort du dossier, un format ancien migré, le résultat ouvert dans la page, une vraie session `claude -p`.
- `docs/architecture/` dit encore que la page possède toujours le projet : à mettre à jour après la décision du fondateur.

## Décisions en attente (#821)

Propriété du projet hors page et texte de la règle cœur ; refus de `project.new` et `project.open` (création par `faulk new`) ; export et analyse servis aux agents sans confirmation ; auteur `agent:<nom>` ; `--allow-network` à la place de `--allow-destructive` ; pas de verrou dans un premier temps ; packs hors périmètre ; et le sort du prototype : fusionné tel quel, ou gardé comme référence pour #823 à #830.
