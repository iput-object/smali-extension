.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 2
    iget-object v0, v1, Llib/Child;->missing:I
#                       ^^^^^^^^^^^^^^^^^^^^^^ S303
    iget-object v0, v1, Llib/Child;->name:Ljava/lang/String;
    sget v0, Llib/Child;->COUNT:I
    iget v0, v1, Llib/Orphan;->x:I
    return-void
.end method
