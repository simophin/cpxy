#include <unistd.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <string.h>
#include <netinet/in.h>
#include <netinet/ip6.h>

ssize_t do_recv_with_orig_dst(
    int fd,
    void *out,
    size_t out_len,
    void *src_addr_buf,
    socklen_t *src_addr_len,
    void *dst_addr_buf,
    socklen_t *dst_addr_len)
{
    uint8_t cmsg_buf[CMSG_SPACE(sizeof(struct sockaddr_in6))];

    struct iovec vec = {
        .iov_base = out,
        .iov_len = out_len,
    };
    struct msghdr hdr = {
        .msg_iov = &vec,
        .msg_iovlen = 1,
        .msg_flags = 0,
        .msg_name = src_addr_buf,
        .msg_namelen = *src_addr_len,
        .msg_control = &cmsg_buf,
        .msg_controllen = sizeof(cmsg_buf),
    };

    ssize_t rc = recvmsg(fd, &hdr, MSG_DONTWAIT);

    if (rc < 0)
    {
        return rc;
    }

    *dst_addr_len = 0;
    struct cmsghdr *cmsg = CMSG_FIRSTHDR(&hdr);
    while (cmsg)
    {
        if ((cmsg->cmsg_level == SOL_IP && cmsg->cmsg_type == IP_ORIGDSTADDR) ||
            (cmsg->cmsg_level == SOL_IPV6 && cmsg->cmsg_type == IPV6_ORIGDSTADDR))
        {
            memcpy(dst_addr_buf, CMSG_DATA(cmsg), cmsg->cmsg_len);
            *dst_addr_len = cmsg->cmsg_len;
            break;
        }

        cmsg = CMSG_NXTHDR(&hdr, cmsg);
    }

    *src_addr_len = hdr.msg_namelen;
    memcpy(src_addr_buf, hdr.msg_name, hdr.msg_namelen);
    return rc;
}