.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 2
    const/high16 v0, 0x7f010000
    const/high16 v0, 0x7f010001
#                    ^^^^^^^^^^ S212
    const-wide/high16 v0, 0x7ff0000000000000L
    const-wide/high16 v0, 0x7ff0000000000001L
#                         ^^^^^^^^^^^^^^^^^^^ S212
    return-void
.end method
