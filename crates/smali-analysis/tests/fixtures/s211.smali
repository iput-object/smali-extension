.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 2
    const/4 v0, 0x7
    const/4 v0, -0x8
    const/4 v0, 0xf
#               ^^^ S211
    const/4 v0, -9
#               ^^ S211
    const/4 v0, 017
#               ^^^ S211
    const/16 v0, 0xffff
    const/16 v0, 65536
#                ^^^^^ S211
    add-int/lit8 v0, v0, 0xff
#                        ^^^^ S211
    add-int/lit8 v0, v0, -128
    const v0, 0xffffffff
    const v0, 0x100000000
#             ^^^^^^^^^^^ S211
    const-wide v0, 0xffffffffffffffffL
    const/4 v0, 'a'
#               ^^^ S211
    const/16 v0, 'a'
    return-void
.end method
