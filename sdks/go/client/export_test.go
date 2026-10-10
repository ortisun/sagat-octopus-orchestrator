package client

// SetMaxAnswer lowers the bound on an answer, for a test; it returns what restores it.
func SetMaxAnswer(n int) (restore func()) {
	old := maxAnswer
	maxAnswer = n
	return func() { maxAnswer = old }
}
