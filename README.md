# Queued rwlock implementation in Rust

This read-write lock uses ticket mutex as waitqueue, which acts
like FIFO. It allows to avoid unfairness and starvation of readers or
writes, that is common problem for generic rwlocks (read-preffered or
write-preffered)

