.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 2
    sget-object v0, Llib/Base;->name:Ljava/lang/String;
#                   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ S305
    iget v0, v1, Llib/Base;->COUNT:I
#                ^^^^^^^^^^^^^^^^^^^ S305
    sget v0, Llib/Base;->COUNT:I
    iget-object v0, v1, Llib/Base;->name:Ljava/lang/String;
    return-void
.end method
