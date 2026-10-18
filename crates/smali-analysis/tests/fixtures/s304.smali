.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 4
    invoke-virtual {v0, v1, v2, v3}, Llib/Base;->make(IJ)Llib/Base;
#                                    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ S304
    invoke-virtual {v0}, Llib/Base;->secret()V
    invoke-interface {v0}, Llib/Base;->run()V
#                          ^^^^^^^^^^^^^^^^^^ S304
    invoke-static {}, Llib/Base;->run()V
#                     ^^^^^^^^^^^^^^^^^^ S304
    invoke-static {v0, v1, v2}, Llib/Base;->make(IJ)Llib/Base;
    invoke-direct {v0}, Llib/Base;->secret()V
    invoke-direct {v0}, Llib/Base;-><init>()V
    invoke-interface {v0}, Llib/Iface;->act()V
    invoke-virtual {v0}, Llib/Child;->act()V
    invoke-super {v0}, Llib/Base;->run()V
    return-void
.end method
