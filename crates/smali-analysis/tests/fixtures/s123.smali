.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 1
    packed-switch v0, :sparse
#                     ^^^^^^^ S123
    sparse-switch v0, :sparse
    fill-array-data v0, :arr
    return-void
    :sparse
    .sparse-switch
        0x1 -> :sparse
    .end sparse-switch
    :arr
    .array-data 4
        0x1
    .end array-data
.end method
