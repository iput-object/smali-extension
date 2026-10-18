.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 2
    const/4 v0, v1
#   ^^^^^^^^^^^^^^ S210
    move v0
#   ^^^^^^^ S210
    const-string v0, 0x1
#   ^^^^^^^^^^^^^^^^^^^^ S210
    move v0, v1
    return-void
.end method
