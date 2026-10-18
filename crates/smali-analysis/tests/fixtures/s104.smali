.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 2
    iget-quick v0, v1, field@0x10
#   ^^^^^^^^^^ S104
    return-void
.end method

.method public g()V
    .registers 2
    frobnicate v0, v1
#   ^^^^^^^^^^ S104
    return-void
.end method
