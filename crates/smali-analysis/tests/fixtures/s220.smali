.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 6
    invoke-static {v0, v1, v2, v3, v4, v5}, Llib/Base;->g(IIIIII)V
#                 ^^^^^^^^^^^^^^^^^^^^^^^^ S220
    invoke-static {v0, v1, v2, v3, v4}, Llib/Base;->g(IIIII)V
    return-void
.end method
