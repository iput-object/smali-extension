.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 4
    invoke-static {v0, v1, v2}, Llib/Base;->make(IJ)Llib/Base;
    .line 3
    move-result-object v0
    const/4 v1, 0x0
    move-result v1
#   ^^^^^^^^^^^^^^ S230
    return-void
.end method
