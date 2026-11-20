# sdk

## Overview

Java library artifact and its production source tree. [pom.xml:sagat-octopus-orchestrator-sdk]

## Interface

- Maven artifact dev.sagatoctopusorchestrator:sagat-octopus-orchestrator-sdk declares automatic module name dev.sagatoctopusorchestrator.sdk. [pom.xml:artifactId] [pom.xml:Automatic-Module-Name]

- [src](./src/README.md) — component contract.

## Invariants

- Compilation targets Java release 17; protobuf/grpc-java generation uses parent-pinned versions. [../pom.xml:maven.compiler.release] [pom.xml:protocArtifact] [pom.xml:pluginArtifact]

## Side Effects

- Maven generates Java protobuf/gRPC sources, compiles classes and packages the JAR when invoked. [pom.xml:protobuf-maven-plugin] [pom.xml:maven-jar-plugin]

## Verification

Run from the repository root:

```sh
cd sdks/java && ./mvnw -q verify
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
