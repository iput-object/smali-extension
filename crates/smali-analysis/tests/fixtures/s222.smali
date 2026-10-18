.class public LT;
.super Ljava/lang/Object;

.method public static f()V
    .locals 4
    invoke-static {v0}, Llib/Base;->make(IJ)Llib/Base;
#   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ S222
    invoke-static {v0, v1, v2}, Llib/Base;->make(IJ)Llib/Base;
    invoke-virtual/range {v0 .. v1}, Llib/Base;->run()V
#   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ S222
    invoke-virtual {v0}, Llib/Base;->run()V
    return-void
.end method
