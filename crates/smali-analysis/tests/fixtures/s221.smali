.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 4
    invoke-static/range {v3 .. v1}, Llib/Base;->g(III)V
#                       ^^^^^^^^^^ S221
    invoke-static/range {v1 .. v3}, Llib/Base;->g(III)V
    return-void
.end method
