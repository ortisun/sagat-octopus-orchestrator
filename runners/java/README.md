# Java inline runner

The builder accepts one top-level `Script.java`. `Script` must be public, have a public no-argument
constructor and implement `sagat.inline.InlineScript`. The input and result are Java maps, lists,
strings, booleans, numbers or null. `Octopus` provides mediated fetch, secret, log and the stable
step idempotency key.

The trusted builder runs `javac --release 17 -proc:none` with explicit arguments. It does not run a
project, Maven, Gradle, an annotation processor or options from the author. Lockfiles and external
dependencies are refused. The v3 artifact contains only `Script.class` and `Script$*.class`. The
runtime decodes them in memory with a parent-first loader, so the signed trusted API and pinned
Jackson JSON implementation cannot be replaced by the script. The JVM verifier remains the final
class-file validator; the container is the security boundary.
