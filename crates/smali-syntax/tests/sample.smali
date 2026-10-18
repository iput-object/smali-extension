.class public final Lcom/foo/Bar$Inner;
.super Ljava/lang/Object;
.source "Bar.java"
.implements Ljava/lang/Runnable;

.field private static final TAG:Ljava/lang/String; = "x"
.field public count:I

.method public constructor <init>()V
    .registers 2
    invoke-direct {p0}, Ljava/lang/Object;-><init>()V
    return-void
.end method

.method public run(IJ)Z
    .locals 3
    .param p1, "a"    # I
    .line 10
    const/4 v0, 0x1
    const-wide/16 v1, 0x10L
    if-eqz p1, :cond_0
    iget v0, p0, Lcom/foo/Bar$Inner;->count:I
    invoke-static {v0, v1}, Lcom/foo/X;->m(IJ)V
    invoke-virtual/range {v0 .. v2}, Lcom/foo/X;->n([I)Ljava/lang/String;
    move-result-object v0
    packed-switch v0, :pswitch_data_0
    :cond_0
    :try_start_0
    goto :cond_0
    :try_end_0
    .catch Ljava/lang/Exception; {:try_start_0 .. :try_end_0} :cond_0
    :pswitch_data_0
    .packed-switch 0x0
        :cond_0
    .end packed-switch
    return v0
.end method

.method public abstract foo()V
.end method
