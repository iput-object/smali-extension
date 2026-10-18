.class public LT;
.super Ljava/lang/Object;

.method public f()V
    .registers 1
    goto :nowhere
#        ^^^^^^^^ S120
    :there
    goto :there
.end method
