# DependencyInjection

## Overview

Microsoft.Extensions service registration, scoped handlers and hosted worker lifecycle. [ServiceCollectionExtensions.cs:AddSagatOctopusOrchestrator]

## Interface

- Public interfaces are `IActivity<TInput,TOutput>`, `IReconciler<TInput>` and ISagatBuilder; SagatServiceCollectionExtensions supplies AddSagatOctopusOrchestrator plus delegate/class AddActivity/AddCompensation/AddReconciler overloads. [Handlers.cs:IActivity] [Handlers.cs:IReconciler] [Handlers.cs:ISagatBuilder] [ServiceCollectionExtensions.cs:SagatServiceCollectionExtensions]
- SagatWorkerService and SagatWorkerRegistrations are internal lifecycle/registration boundaries. [SagatWorkerService.cs:SagatWorkerService] [SagatWorkerService.cs:SagatWorkerRegistrations]

## Invariants

- A worker/hosted service is registered when the first handler is added; absent worker names use MachineName. Class handlers must implement one matching generic contract and receive a fresh async scope per task. [ServiceCollectionExtensions.cs:RegistrationsOf] [ServiceCollectionExtensions.cs:ContractOf]

## Side Effects

- Mutates IServiceCollection, creates scopes and starts/stops IWorkerHost with application hosting. [ServiceCollectionExtensions.cs:AddSagatOctopusOrchestrator] [SagatWorkerService.cs:StartAsync]

## Verification

Run from the repository root:

```sh
cd sdks/dotnet && dotnet test && dotnet format --verify-no-changes
```

These commands are documented from the existing build configuration; they were not executed during this documentation update.
