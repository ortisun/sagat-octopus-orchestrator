package worker

import "time"

// ObservePauses makes observe see every pause of the loop of h, with its length, as the pause begins. It is called
// before Start; the tests read the pacing of the host from it instead of from the wall clock (AGENTS.md Rule 18).
func ObservePauses(h *Host, observe func(time.Duration)) { h.pauses = observe }
