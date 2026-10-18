.class public LT;
.super Ljava/lang/Object;

.method public f(I)V
    .locals 20
    const/4 v15, 0x0
    const/4 v16, 0x0
#           ^^^ S204
    const/4 p1, 0x0
#           ^^ S204
    move-object/from16 v0, p1
    return-void
.end method
