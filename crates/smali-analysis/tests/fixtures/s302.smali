.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 2
    invoke-virtual {v0}, Llib/Child;->nope()V
#                        ^^^^^^^^^^^^^^^^^^^^ S302
    invoke-virtual {v0}, Llib/Child;->run()V
    invoke-virtual {v0}, Llib/Orphan;->nope()V
    invoke-virtual {v0}, Lnot/There;->nope()V
    invoke-virtual {v0}, Ljava/lang/Object;->hashCode()I
    return-void
.end method
