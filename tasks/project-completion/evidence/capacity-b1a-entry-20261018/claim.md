CLAIM: One exact existing group acquired with NOWAIT after existing chain/own-run entry, with
mandatory whole-TX rollback on contention, can bound the new group-entry mechanism without a
blocking group acquisition edge. It does not establish production transition authority or remove
existing suffix/implicit waits.
WHY IT MATTERS: A blocking group added to only some run/ancestry prefixes can introduce cycles;
retrying a failed try while retaining run/chain guards would recreate that waiting dependency.
