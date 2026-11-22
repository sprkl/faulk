2026-11-05
* **Update** (#763): the engine test `graph_cost::an_empty_track_costs_no_notable_computation` no longer reads the machine's load: the three engines are timed in turn inside each of 15 rounds (best per engine), and it asserts the empty-track / playing-track cost ratio only, with no bound on a share of the wall-clock budget (printed instead).
