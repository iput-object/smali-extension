.class public LT;
.super Ljava/lang/Object;

.method public static v()V
    .locals 1
    return v0
#   ^^^^^^ S231
.end method

.method public static i()I
    .locals 1
    const/4 v0, 0x0
    return v0
.end method

.method public static j()J
    .locals 2
    return-object v0
#   ^^^^^^^^^^^^^ S231
.end method

.method public static o()Ljava/lang/Object;
    .locals 1
    return-object v0
.end method
